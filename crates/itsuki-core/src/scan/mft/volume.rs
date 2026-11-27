//! E/S del motor MFT: abre el volumen, localiza la MFT y la lee en bloques
//! grandes y secuenciales, analizando cada bloque en paralelo mientras se lee
//! el siguiente.

use std::alloc::{Layout, alloc_zeroed, dealloc};
use std::io;
use std::path::Path;
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::sync::mpsc::{channel, sync_channel};
use std::time::{Duration, Instant};

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
use crate::scan::util::{Handle, Perf, verbatim};
use crate::scan::walk::WalkOutput;

/// Bytes de MFT que se leen por bloque.
const CHUNK: usize = 4 << 20;
/// Registros por tarea de análisis.
const RECORDS_PER_TASK: usize = 1024;
/// Tamaño mínimo de lectura al que se baja si el controlador rechaza las grandes.
const MIN_IO: usize = 64 << 10;
/// Lectores en paralelo en unidades SSD (en discos mecánicos se usa uno).
const READERS: usize = 4;

/// Geometría del volumen NTFS.
struct Geometry {
    sector: u64,
    cluster: u64,
    record: u64,
    mft_start_lcn: u64,
    mft_valid: u64,
}

/// Volumen abierto para lectura directa.
struct Volume {
    handle: Handle,
    /// Tamaño máximo de cada lectura. Algunos controladores (sobre todo de
    /// discos SATA) rechazan lecturas grandes: entonces se va reduciendo.
    max_io: AtomicUsize,
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

/// Añade al error el paso en el que ocurrió.
fn step(what: &'static str) -> impl FnOnce(io::Error) -> io::Error {
    move |e| io::Error::new(e.kind(), format!("{what}: {e}"))
}

/// Forma de abrir el volumen: ruta del dispositivo (terminada en NUL) y si se
/// usa la caché del sistema.
#[derive(Clone)]
struct Access {
    path: Vec<u16>,
    buffered: bool,
}

fn open_volume(access: &Access) -> io::Result<Volume> {
    let flags = if access.buffered { 0 } else { FILE_FLAG_NO_BUFFERING };
    // SAFETY: `path` termina en NUL.
    let handle = Handle::from_create(unsafe {
        CreateFileW(
            access.path.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null(),
            OPEN_EXISTING,
            flags,
            ptr::null_mut(),
        )
    })?;
    Ok(Volume { handle, max_io: AtomicUsize::new(CHUNK) })
}

/// Formas de abrir el volumen, de la más rápida a la más compatible: por su
/// nombre GUID y por su letra, sin caché y con caché. Algunos controladores
/// (por ejemplo, de ciertos discos SATA) rechazan la lectura directa sin caché.
fn access_candidates(mount: &[u16], device: &[u16]) -> Vec<Access> {
    let mut paths = vec![device.to_vec()];
    let end = mount.iter().position(|&c| c == 0).unwrap_or(mount.len());
    let mount = String::from_utf16_lossy(&mount[..end]);
    let mount = mount.strip_prefix(r"\\?\").unwrap_or(&mount);
    let b = mount.as_bytes();
    if b.len() == 3 && b[0].is_ascii_alphabetic() && &b[1..] == b":\\" {
        paths.push(format!(r"\\.\{}", &mount[..2]).encode_utf16().chain(Some(0)).collect());
    }
    [false, true]
        .into_iter()
        .flat_map(|buffered| paths.iter().map(move |p| Access { path: p.clone(), buffered }))
        .collect()
}

/// Las bibliotecas de juegos de Xbox (volúmenes con `.GamingRoot`) tienen un
/// filtro de Gaming Services que rechaza la lectura directa del volumen.
fn explain_blocked(mount: &[u16], error: io::Error) -> io::Error {
    let end = mount.iter().position(|&c| c == 0).unwrap_or(mount.len());
    let root = String::from_utf16_lossy(&mount[..end]);
    let gaming = Path::new(&root).join(".GamingRoot").exists();
    if error.raw_os_error() == Some(50) || error.to_string().contains("os error 50") {
        if gaming {
            return io::Error::new(
                io::ErrorKind::Unsupported,
                "el volumen tiene juegos de Xbox y su filtro de protección (gameflt) bloquea la lectura directa; \
                 usa el motor nativo",
            );
        }
        return io::Error::new(
            io::ErrorKind::Unsupported,
            format!("un controlador o filtro del sistema bloquea la lectura directa del volumen ({error})"),
        );
    }
    error
}
/// Escanea `root` leyendo directamente la MFT de su volumen.
pub(crate) fn scan(
    root: &Path,
    pool: &rayon::ThreadPool,
    progress: &ScanProgress,
    keep_ids: bool,
) -> io::Result<WalkOutput> {
    let mut perf = Perf::new("mft");
    let root_w = verbatim(root);
    let mut mount = vec![0u16; 1024];
    // SAFETY: búferes válidos con sus tamaños.
    if unsafe { GetVolumePathNameW(root_w.as_ptr(), mount.as_mut_ptr(), mount.len() as u32) } == 0 {
        return Err(step("ruta del volumen")(io::Error::last_os_error()));
    }
    let mut device = vec![0u16; 64];
    // SAFETY: `mount` termina en NUL; el tamaño coincide con el búfer.
    if unsafe { GetVolumeNameForVolumeMountPointW(mount.as_ptr(), device.as_mut_ptr(), device.len() as u32) } == 0 {
        return Err(step("nombre del volumen")(io::Error::last_os_error()));
    }
    // "\\?\Volume{...}\" -> "\\?\Volume{...}" (sin la barra final se abre el volumen).
    let end = device.iter().position(|&c| c == 0).unwrap_or(device.len());
    if end > 0 && device[end - 1] == u16::from(b'\\') {
        device[end - 1] = 0;
    }
    device.truncate(end.max(1));
    if device.last() != Some(&0) {
        device.push(0);
    }

    // Se prueba cada forma de abrir el volumen hasta que una lee la MFT.
    let mut last_error = None;
    let mut chosen = None;
    for access in access_candidates(&mount, &device) {
        let attempt = open_volume(&access).map_err(step("abrir el volumen")).and_then(|v| {
            let geo = geometry(&v).map_err(step("datos del volumen NTFS"))?;
            let runs = mft_runs(&v, &geo).map_err(step("ubicación de la MFT"))?;
            Ok((v, geo, runs))
        });
        match attempt {
            Ok(found) => {
                chosen = Some((access, found));
                break;
            }
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => return Err(e),
            Err(e) => last_error = Some(e),
        }
    }
    let Some((access, (volume, geo, runs))) = chosen else {
        let error = last_error.unwrap_or_else(|| invalid("no se pudo abrir el volumen"));
        return Err(explain_blocked(&mount, error));
    };
    let root_frn = file_reference(&root_w).map_err(step("referencia de la carpeta"))? & FRN_MASK;
    // En discos mecánicos las lecturas en paralelo obligan a mover el cabezal.
    let hdd = crate::system::volume_of(root).and_then(|v| v.seek_penalty).unwrap_or(false);
    let readers = if hdd { 1 } else { READERS };
    perf.mark(&format!(
        "abrir volumen ({}, {} lector(es)) y localizar la MFT",
        if access.buffered { "con caché" } else { "sin caché" },
        readers
    ));

    let total_records = (geo.mft_valid / geo.record) as usize;
    if root_frn as usize >= total_records {
        return Err(invalid("la carpeta raíz está fuera de la tabla"));
    }
    let mut recs = vec![RecInfo::default(); total_records];
    let mut all = ParseOut::default();
    let (read_time, parse_time) =
        read_and_parse(volume, &access, readers, &geo, &runs, &mut recs, &mut all, pool, progress)
            .map_err(step("lectura de la MFT"))?;
    perf.mark(&format!(
        "leer y analizar {} registros ({} MB; lectura {:.0} ms, análisis {:.0} ms)",
        total_records,
        geo.mft_valid >> 20,
        read_time.as_secs_f64() * 1000.0,
        parse_time.as_secs_f64() * 1000.0
    ));
    if progress.is_cancelled() {
        return Ok(WalkOutput { nodes: Vec::new(), names: Vec::new(), ids: None, denied: Vec::new() });
    }
    apply_extensions(&mut recs, &all.ext);
    let ParseOut { links, names, .. } = all;
    let out = build(&recs, &links, names, root_frn as u32, keep_ids);
    perf.mark("construir el árbol");
    Ok(out)
}
/// Pide a NTFS el tamaño de sector, clúster y registro, y dónde está la MFT.
fn geometry(volume: &Volume) -> io::Result<Geometry> {
    let mut data = NTFS_VOLUME_DATA_BUFFER::default();
    let mut returned = 0u32;
    // SAFETY: búfer de salida válido con su tamaño.
    let ok = unsafe {
        DeviceIoControl(
            volume.handle.0,
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

/// Una sola llamada a `ReadFile` en la posición `offset`.
fn read_once(volume: &Volume, offset: u64, buf: &mut [u8]) -> io::Result<()> {
    let mut ov = OVERLAPPED {
        Anonymous: OVERLAPPED_0 {
            Anonymous: OVERLAPPED_0_0 { Offset: offset as u32, OffsetHigh: (offset >> 32) as u32 },
        },
        ..Default::default()
    };
    let mut read = 0u32;
    // SAFETY: búfer válido; la lectura es síncrona porque el handle no es OVERLAPPED.
    let ok = unsafe { ReadFile(volume.handle.0, buf.as_mut_ptr(), buf.len() as u32, &mut read, &mut ov) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    if read as usize != buf.len() {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "lectura incompleta del volumen"));
    }
    Ok(())
}

/// Lectura posicionada de `buf.len()` bytes (alineados al sector). Si el
/// controlador rechaza lecturas tan grandes, se repite en trozos más pequeños.
fn read_at(volume: &Volume, offset: u64, buf: &mut [u8]) -> io::Result<()> {
    let mut done = 0usize;
    while done < buf.len() {
        let max = volume.max_io.load(Relaxed);
        let len = (buf.len() - done).min(max);
        let at = offset + done as u64;
        match read_once(volume, at, &mut buf[done..done + len]) {
            Ok(()) => done += len,
            // ERROR_NOT_SUPPORTED, ERROR_INVALID_PARAMETER, ERROR_NO_SYSTEM_RESOURCES, ERROR_INVALID_USER_BUFFER
            Err(e) if matches!(e.raw_os_error(), Some(50 | 87 | 1450 | 1784)) && len > MIN_IO => {
                volume.max_io.store((len / 2).max(MIN_IO), Relaxed);
            }
            Err(e) => return Err(io::Error::new(e.kind(), format!("{len} bytes en la posición {at}: {e}"))),
        }
    }
    Ok(())
}

/// Lee los clústeres `vcn..vcn+count` de la MFT (según `runs`) en `buf`.
fn read_vcns(volume: &Volume, geo: &Geometry, runs: &[Run], vcn: u64, count: u64, buf: &mut [u8]) -> io::Result<()> {
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
fn read_record(volume: &Volume, geo: &Geometry, runs: &[Run], frn: u64) -> io::Result<Vec<u8>> {
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
fn mft_runs(volume: &Volume, geo: &Geometry) -> io::Result<Vec<Run>> {
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
fn read_attribute(volume: &Volume, geo: &Geometry, runs: &[Run], size: u64) -> io::Result<Vec<u8>> {
    let mut out = Vec::with_capacity(size as usize);
    for r in runs {
        let bytes = (r.len * geo.cluster) as usize;
        let mut buf = AlignedBuf::new(bytes);
        let slice = &mut buf.as_mut()[..bytes];
        read_at(volume, r.lcn * geo.cluster, slice)?;
        out.extend_from_slice(slice);
    }
    out.truncate(size as usize);
    Ok(out)
}

/// Lee la MFT entera en bloques de 4 MiB. Varios hilos lectores (cada uno
/// con su propio handle, para tener varias lecturas en vuelo) envían los
/// bloques a este hilo, que los analiza en paralelo con rayon. Devuelve el
/// tiempo de lectura y el de análisis.
#[allow(clippy::too_many_arguments)]
fn read_and_parse(
    first: Volume,
    access: &Access,
    readers: usize,
    geo: &Geometry,
    runs: &[Run],
    recs: &mut [RecInfo],
    all: &mut ParseOut,
    pool: &rayon::ThreadPool,
    progress: &ScanProgress,
) -> io::Result<(Duration, Duration)> {
    let record = geo.record as usize;
    let cluster = geo.cluster;
    let total = recs.len() as u64 * geo.record;
    let chunks = total.div_ceil(CHUNK as u64) as usize;
    let readers = readers.clamp(1, chunks.max(1));
    let chunk_clusters = CHUNK as u64 / cluster;
    // Resultados por bloque: se unen en orden para que el árbol sea siempre igual.
    let mut outs: Vec<Option<ParseOut>> = (0..chunks).map(|_| None).collect();
    let mut first = Some(first);

    let times = std::thread::scope(|s| -> io::Result<(Duration, Duration)> {
        let (full_tx, full_rx) = sync_channel::<(usize, io::Result<(usize, AlignedBuf)>)>(readers * 2);
        let mut free = Vec::with_capacity(readers);
        let mut workers = Vec::with_capacity(readers);
        for r in 0..readers {
            let (free_tx, free_rx) = channel::<AlignedBuf>();
            for _ in 0..2 {
                free_tx.send(AlignedBuf::new(CHUNK)).expect("canal abierto");
            }
            free.push(free_tx);
            let full_tx = full_tx.clone();
            let own = first.take();
            workers.push(s.spawn(move || {
                let mut busy = Duration::ZERO;
                let volume = match own.map_or_else(|| open_volume(access), Ok) {
                    Ok(v) => v,
                    Err(e) => {
                        let _ = full_tx.send((r, Err(e)));
                        return busy;
                    }
                };
                for k in (r..chunks).step_by(readers) {
                    if progress.is_cancelled() {
                        break;
                    }
                    let Ok(mut buf) = free_rx.recv() else { break };
                    let offset = k as u64 * CHUNK as u64;
                    let clusters = (total - offset).min(CHUNK as u64).div_ceil(cluster).min(chunk_clusters);
                    let t = Instant::now();
                    let result =
                        read_vcns(&volume, geo, runs, offset / cluster, clusters, buf.as_mut()).map(|()| (k, buf));
                    busy += t.elapsed();
                    let failed = result.is_err();
                    if full_tx.send((r, result)).is_err() || failed {
                        break;
                    }
                }
                busy
            }));
        }
        drop(full_tx);

        let mut parse_time = Duration::ZERO;
        for (r, message) in full_rx {
            let (k, mut buf) = message?;
            let t = Instant::now();
            let offset = k as u64 * CHUNK as u64;
            let len = (total - offset).min(CHUNK as u64) as usize;
            let first_rec = (offset / geo.record) as usize;
            let count = len / record;
            let data = &mut buf.as_mut()[..count * record];
            let slots = &mut recs[first_rec..first_rec + count];
            let parts: Vec<ParseOut> = pool.install(|| {
                data.par_chunks_mut(RECORDS_PER_TASK * record)
                    .zip(slots.par_chunks_mut(RECORDS_PER_TASK))
                    .enumerate()
                    .map(|(i, (d, r))| {
                        let mut out = ParseOut::default();
                        parse_records(d, record, cluster, (first_rec + i * RECORDS_PER_TASK) as u64, r, &mut out);
                        out
                    })
                    .collect()
            });
            let mut merged = ParseOut::default();
            for part in parts {
                progress.add(part.files, part.dirs, part.bytes);
                merged.append(part);
            }
            outs[k] = Some(merged);
            parse_time += t.elapsed();
            let _ = free[r].send(buf);
        }
        let read_time = workers.into_iter().map(|w| w.join().unwrap_or_default()).max().unwrap_or_default();
        Ok((read_time, parse_time))
    })?;
    for out in outs.into_iter().flatten() {
        all.append(out);
    }
    Ok(times)
}
