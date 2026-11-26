//! E/S del motor MFT: abre el volumen, localiza la MFT y la lee en bloques
//! grandes y secuenciales, analizando cada bloque en paralelo mientras se lee
//! el siguiente.

use std::alloc::{Layout, alloc_zeroed, dealloc};
use std::io;
use std::path::Path;
use std::ptr;
use std::sync::mpsc::{channel, sync_channel};

use rayon::prelude::*;
use windows_sys::Win32::Foundation::GENERIC_READ;
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_NO_BUFFERING, FILE_READ_ATTRIBUTES,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, GetFileInformationByHandle,
    GetVolumeNameForVolumeMountPointW, GetVolumePathNameW, OPEN_EXISTING, ReadFile,
};
use windows_sys::Win32::System::IO::{DeviceIoControl, OVERLAPPED, OVERLAPPED_0, OVERLAPPED_0_0};
use windows_sys::Win32::System::Ioctl::{FSCTL_GET_NTFS_VOLUME_DATA, NTFS_VOLUME_DATA_BUFFER};

use super::build::{apply_extensions, build};
use super::parse::{
    ATTR_ATTRIBUTE_LIST, ATTR_DATA, FRN_MASK, ParseOut, RecInfo, Run, apply_fixup, attributes, data_segments_in_list,
    parse_records,
};
use crate::scan::ScanProgress;
use crate::scan::util::{Handle, verbatim};
use crate::scan::walk::WalkOutput;

/// Bytes de MFT que se leen por bloque.
const CHUNK: usize = 4 << 20;
/// Registros por tarea de análisis.
const RECORDS_PER_TASK: usize = 1024;

/// Geometría del volumen NTFS.
struct Geometry {
    sector: u64,
    cluster: u64,
    record: u64,
    mft_start_lcn: u64,
    mft_valid: u64,
}

/// Búfer alineado a 4 KiB (requisito de la lectura sin caché).
struct AlignedBuf {
    ptr: *mut u8,
    layout: Layout,
}

// SAFETY: el búfer es memoria propia y no compartida.
unsafe impl Send for AlignedBuf {}

impl AlignedBuf {
    fn new(len: usize) -> AlignedBuf {
        let layout = Layout::from_size_align(len.max(4096), 4096).expect("tamaño de búfer válido");
        // SAFETY: el tamaño no es cero.
        let ptr = unsafe { alloc_zeroed(layout) };
        if ptr.is_null() {
            std::alloc::handle_alloc_error(layout);
        }
        AlignedBuf { ptr, layout }
    }

    fn as_mut(&mut self) -> &mut [u8] {
        // SAFETY: memoria propia de `layout.size()` bytes.
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.layout.size()) }
    }
}

impl Drop for AlignedBuf {
    fn drop(&mut self) {
        // SAFETY: se reservó con este mismo layout.
        unsafe { dealloc(self.ptr, self.layout) };
    }
}

fn invalid(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("MFT: {msg}"))
}

/// Escanea `root` leyendo directamente la MFT de su volumen.
pub(crate) fn scan(
    root: &Path,
    pool: &rayon::ThreadPool,
    progress: &ScanProgress,
    keep_ids: bool,
) -> io::Result<WalkOutput> {
    let root_w = verbatim(root);
    let mut mount = vec![0u16; 1024];
    // SAFETY: búferes válidos con sus tamaños.
    if unsafe { GetVolumePathNameW(root_w.as_ptr(), mount.as_mut_ptr(), mount.len() as u32) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut device = vec![0u16; 64];
    // SAFETY: `mount` termina en NUL; el tamaño coincide con el búfer.
    if unsafe { GetVolumeNameForVolumeMountPointW(mount.as_ptr(), device.as_mut_ptr(), device.len() as u32) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // "\\?\Volume{...}\" -> "\\?\Volume{...}" (sin la barra final se abre el volumen).
    let end = device.iter().position(|&c| c == 0).unwrap_or(device.len());
    if end > 0 && device[end - 1] == u16::from(b'\\') {
        device[end - 1] = 0;
    }

    // SAFETY: `device` termina en NUL.
    let volume = Handle::from_create(unsafe {
        CreateFileW(
            device.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_NO_BUFFERING,
            ptr::null_mut(),
        )
    })?;

    let geo = geometry(&volume)?;
    let root_frn = file_reference(&root_w)? & FRN_MASK;
    let runs = mft_runs(&volume, &geo)?;

    let total_records = (geo.mft_valid / geo.record) as usize;
    if root_frn as usize >= total_records {
        return Err(invalid("la carpeta raíz está fuera de la tabla"));
    }
    let mut recs = vec![RecInfo::default(); total_records];
    let mut all = ParseOut::default();
    read_and_parse(&volume, &geo, &runs, &mut recs, &mut all, pool, progress)?;
    if progress.is_cancelled() {
        return Ok(WalkOutput { nodes: Vec::new(), names: Vec::new(), ids: None, denied: Vec::new() });
    }
    apply_extensions(&mut recs, &all.ext);
    let ParseOut { links, names, .. } = all;
    Ok(build(&recs, &links, names, root_frn as u32, keep_ids))
}

/// Pide a NTFS el tamaño de sector, clúster y registro, y dónde está la MFT.
fn geometry(volume: &Handle) -> io::Result<Geometry> {
    let mut data = NTFS_VOLUME_DATA_BUFFER::default();
    let mut returned = 0u32;
    // SAFETY: búfer de salida válido con su tamaño.
    let ok = unsafe {
        DeviceIoControl(
            volume.0,
            FSCTL_GET_NTFS_VOLUME_DATA,
            ptr::null(),
            0,
            (&mut data as *mut NTFS_VOLUME_DATA_BUFFER).cast(),
            size_of::<NTFS_VOLUME_DATA_BUFFER>() as u32,
            &mut returned,
            ptr::null_mut(),
        )
    };
    if ok == 0 {
        // Sin NTFS (FAT, exFAT, ReFS…) esta llamada falla.
        return Err(io::Error::last_os_error());
    }
    let geo = Geometry {
        sector: u64::from(data.BytesPerSector),
        cluster: u64::from(data.BytesPerCluster),
        record: u64::from(data.BytesPerFileRecordSegment),
        mft_start_lcn: data.MftStartLcn as u64,
        mft_valid: data.MftValidDataLength as u64,
    };
    let pow2 = |v: u64| v.is_power_of_two();
    if !(pow2(geo.sector) && pow2(geo.cluster) && pow2(geo.record))
        || geo.record < 512
        || geo.cluster > CHUNK as u64
        || geo.sector > geo.cluster
    {
        return Err(invalid("geometría del volumen no soportada"));
    }
    Ok(geo)
}

/// Número de referencia de archivo (registro de la MFT) de una ruta.
fn file_reference(path_w: &[u16]) -> io::Result<u64> {
    // SAFETY: `path_w` termina en NUL.
    let h = Handle::from_create(unsafe {
        CreateFileW(
            path_w.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            ptr::null_mut(),
        )
    })?;
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: estructura de salida válida.
    if unsafe { GetFileInformationByHandle(h.0, &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow))
}

/// Lectura posicionada de `buf.len()` bytes (alineados al sector).
fn read_at(volume: &Handle, offset: u64, buf: &mut [u8]) -> io::Result<()> {
    let mut ov = OVERLAPPED {
        Anonymous: OVERLAPPED_0 {
            Anonymous: OVERLAPPED_0_0 { Offset: offset as u32, OffsetHigh: (offset >> 32) as u32 },
        },
        ..Default::default()
    };
    let mut read = 0u32;
    // SAFETY: búfer válido; la lectura es síncrona porque el handle no es OVERLAPPED.
    let ok = unsafe { ReadFile(volume.0, buf.as_mut_ptr(), buf.len() as u32, &mut read, &mut ov) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    if read as usize != buf.len() {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "lectura incompleta del volumen"));
    }
    Ok(())
}

/// Lee los clústeres `vcn..vcn+count` de la MFT (según `runs`) en `buf`.
fn read_vcns(volume: &Handle, geo: &Geometry, runs: &[Run], vcn: u64, count: u64, buf: &mut [u8]) -> io::Result<()> {
    let mut done = 0u64;
    while done < count {
        let cur = vcn + done;
        let i = runs.partition_point(|r| r.vcn + r.len <= cur);
        let run = runs.get(i).filter(|r| r.vcn <= cur).ok_or_else(|| invalid("faltan tramos de la tabla"))?;
        let take = (run.vcn + run.len - cur).min(count - done);
        let start = (done * geo.cluster) as usize;
        let end = start + (take * geo.cluster) as usize;
        read_at(volume, (run.lcn + (cur - run.vcn)) * geo.cluster, &mut buf[start..end])?;
        done += take;
    }
    Ok(())
}

/// Lee y corrige un registro concreto de la MFT.
fn read_record(volume: &Handle, geo: &Geometry, runs: &[Run], frn: u64) -> io::Result<Vec<u8>> {
    let byte = frn * geo.record;
    let first = byte / geo.cluster;
    let last = (byte + geo.record).div_ceil(geo.cluster);
    let mut buf = AlignedBuf::new(((last - first) * geo.cluster) as usize);
    read_vcns(volume, geo, runs, first, last - first, buf.as_mut())?;
    let at = (byte - first * geo.cluster) as usize;
    let mut rec = buf.as_mut()[at..at + geo.record as usize].to_vec();
    if rec.get(0..4) != Some(b"FILE") || !apply_fixup(&mut rec) {
        return Err(invalid("registro dañado"));
    }
    Ok(rec)
}

/// Tramos de clústeres de la propia MFT, a partir de su registro 0 (y de sus
/// registros de extensión si la MFT está muy fragmentada).
fn mft_runs(volume: &Handle, geo: &Geometry) -> io::Result<Vec<Run>> {
    // El registro 0 está al principio de la MFT: un tramo provisional basta para leerlo.
    let clusters = geo.record.div_ceil(geo.cluster).max(1);
    let bootstrap = [Run { vcn: 0, lcn: geo.mft_start_lcn, len: clusters }];
    let rec0 = read_record(volume, geo, &bootstrap, 0)?;

    let mut runs = Vec::new();
    let mut list: Option<Vec<u8>> = None;
    for a in attributes(&rec0) {
        if a.ty == ATTR_DATA && a.name_len == 0 && a.non_resident {
            runs.extend(a.runs().ok_or_else(|| invalid("tramos ilegibles en $MFT"))?);
        } else if a.ty == ATTR_ATTRIBUTE_LIST {
            list = Some(match a.value() {
                Some(v) => v.to_vec(),
                None => read_attribute(volume, geo, &a.runs().unwrap_or_default(), a.data_size().unwrap_or(0))?,
            });
        }
    }
    if runs.is_empty() {
        return Err(invalid("no se encontró $DATA en $MFT"));
    }
    // MFT fragmentada: el resto de tramos está en registros de extensión.
    if let Some(list) = list {
        for segment in data_segments_in_list(&list) {
            let rec = read_record(volume, geo, &runs, segment)?;
            for a in attributes(&rec) {
                if a.ty == ATTR_DATA && a.name_len == 0 && a.non_resident {
                    runs.extend(a.runs().ok_or_else(|| invalid("tramos ilegibles en una extensión de $MFT"))?);
                }
            }
        }
        runs.sort_unstable_by_key(|r| r.vcn);
        runs.dedup_by_key(|r| r.vcn);
    }
    let covered: u64 = runs.iter().map(|r| r.len).sum::<u64>() * geo.cluster;
    if covered < geo.mft_valid {
        return Err(invalid("los tramos no cubren toda la tabla"));
    }
    Ok(runs)
}

/// Lee un atributo no residente completo (se usa para listas de atributos grandes).
fn read_attribute(volume: &Handle, geo: &Geometry, runs: &[Run], size: u64) -> io::Result<Vec<u8>> {
    let mut out = Vec::with_capacity(size as usize);
    for r in runs {
        let mut buf = AlignedBuf::new((r.len * geo.cluster) as usize);
        let slice = &mut buf.as_mut()[..(r.len * geo.cluster) as usize];
        read_at(volume, r.lcn * geo.cluster, slice)?;
        out.extend_from_slice(slice);
    }
    out.truncate(size as usize);
    Ok(out)
}

/// Lee la MFT entera en bloques de 4 MiB con un hilo lector, mientras los
/// hilos de rayon analizan el bloque anterior.
fn read_and_parse(
    volume: &Handle,
    geo: &Geometry,
    runs: &[Run],
    recs: &mut [RecInfo],
    all: &mut ParseOut,
    pool: &rayon::ThreadPool,
    progress: &ScanProgress,
) -> io::Result<()> {
    let record = geo.record as usize;
    let total = recs.len() as u64 * geo.record;
    let chunk_clusters = CHUNK as u64 / geo.cluster;

    std::thread::scope(|s| -> io::Result<()> {
        let (full_tx, full_rx) = sync_channel::<io::Result<(u64, usize, AlignedBuf)>>(2);
        let (free_tx, free_rx) = channel::<AlignedBuf>();
        for _ in 0..3 {
            free_tx.send(AlignedBuf::new(CHUNK)).expect("canal abierto");
        }
        s.spawn(move || {
            let mut offset = 0u64;
            while offset < total && !progress.is_cancelled() {
                let Ok(mut buf) = free_rx.recv() else { break };
                let len = (total - offset).min(CHUNK as u64);
                let clusters = len.div_ceil(geo.cluster).min(chunk_clusters);
                let result = read_vcns(volume, geo, runs, offset / geo.cluster, clusters, buf.as_mut())
                    .map(|()| (offset, len as usize, buf));
                let failed = result.is_err();
                if full_tx.send(result).is_err() || failed {
                    break;
                }
                offset += len;
            }
        });

        for message in full_rx {
            let (offset, len, mut buf) = message?;
            let first = (offset / geo.record) as usize;
            let count = len / record;
            let data = &mut buf.as_mut()[..count * record];
            let slots = &mut recs[first..first + count];
            let outs: Vec<ParseOut> = pool.install(|| {
                data.par_chunks_mut(RECORDS_PER_TASK * record)
                    .zip(slots.par_chunks_mut(RECORDS_PER_TASK))
                    .enumerate()
                    .map(|(i, (d, r))| {
                        let mut out = ParseOut::default();
                        parse_records(d, record, (first + i * RECORDS_PER_TASK) as u64, r, &mut out);
                        out
                    })
                    .collect()
            });
            for out in outs {
                progress.add(out.files, out.dirs, out.bytes);
                all.append(out);
            }
            let _ = free_tx.send(buf);
        }
        Ok(())
    })
}
