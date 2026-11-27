//! Búsqueda de archivos duplicados.
//!
//! Para leer lo mínimo posible se trabaja en tres pasos:
//! 1. Se agrupan por tamaño (sin leer nada: el árbol ya lo sabe).
//! 2. A los empatados se les calcula un hash del principio y del final (64 KiB
//!    de cada extremo).
//! 3. Solo a los que siguen empatados se les calcula el hash completo
//!    (xxHash3 de 128 bits).
//!
//! Se excluyen los enlaces duros (son el mismo archivo), los archivos que
//! solo están en la nube (leerlos los descargaría) y, salvo que se pida, las
//! carpetas del sistema y de programas, donde los duplicados son normales.

use std::cell::RefCell;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::time::{Duration, Instant};

use rayon::prelude::*;
use serde::Serialize;
use xxhash_rust::xxh3::Xxh3;

use crate::tree::{NodeId, ROOT, Tree, flags};

/// Bytes que se leen de cada extremo en el hash parcial.
const PART: usize = 64 * 1024;
/// Tamaño de bloque del hash completo.
const BLOCK: usize = 1 << 20;

thread_local! {
    static BUF: RefCell<Vec<u8>> = RefCell::new(vec![0u8; BLOCK]);
}

/// Opciones de la búsqueda.
#[derive(Clone, Debug, Serialize)]
pub struct DupOptions {
    /// Tamaño mínimo de los archivos a comparar.
    pub min_size: u64,
    /// Incluir Windows, Archivos de programa, ProgramData, AppData, node_modules y .git.
    pub include_system: bool,
    /// Hilos de lectura (0 = automático: menos en discos mecánicos).
    pub threads: usize,
}

impl Default for DupOptions {
    fn default() -> Self {
        DupOptions { min_size: 1 << 20, include_system: false, threads: 0 }
    }
}

/// Progreso en vivo de la búsqueda.
#[derive(Debug, Default)]
pub struct DupProgress {
    files: AtomicU64,
    bytes: AtomicU64,
    cancel: AtomicBool,
}

impl DupProgress {
    pub fn new() -> Self {
        Self::default()
    }

    /// (archivos leídos, bytes leídos).
    pub fn snapshot(&self) -> (u64, u64) {
        (self.files.load(Relaxed), self.bytes.load(Relaxed))
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Relaxed)
    }
}

/// Archivos idénticos.
#[derive(Clone, Debug, Serialize)]
pub struct DupGroup {
    pub size: u64,
    /// Espacio en disco de cada copia.
    pub alloc: u64,
    /// Rutas ordenadas alfabéticamente.
    pub files: Vec<NodeId>,
}

impl DupGroup {
    /// Espacio que se recupera dejando una sola copia.
    pub fn wasted(&self) -> u64 {
        self.alloc * (self.files.len() as u64).saturating_sub(1)
    }
}

/// Resultado de la búsqueda.
#[derive(Clone, Debug, Default, Serialize)]
pub struct DupReport {
    /// De mayor a menor espacio recuperable.
    pub groups: Vec<DupGroup>,
    /// Archivos que cumplían el filtro.
    pub candidates: u64,
    /// Archivos que hubo que leer (total o parcialmente).
    pub read_files: u64,
    pub read_bytes: u64,
    /// Archivos que no se pudieron leer (en uso, sin permiso…).
    pub errors: u64,
    pub elapsed: Duration,
}

impl DupReport {
    pub fn wasted(&self) -> u64 {
        self.groups.iter().map(DupGroup::wasted).sum()
    }
}

/// Busca archivos duplicados en el árbol.
pub fn find_duplicates(tree: &Tree, opts: &DupOptions, progress: &DupProgress) -> io::Result<DupReport> {
    let start = Instant::now();
    let threads = if opts.threads > 0 {
        opts.threads
    } else if crate::system::volume_of(tree.root_path()).and_then(|v| v.seek_penalty).unwrap_or(false) {
        2
    } else {
        crate::scan::default_threads()
    };
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().map_err(io::Error::other)?;

    let excluded = if opts.include_system { vec![false; tree.len()] } else { excluded_subtrees(tree) };
    let skip = flags::DIR | flags::HARDLINK | flags::CLOUD | flags::REPARSE;
    let mut candidates: Vec<NodeId> = (1..tree.len() as NodeId)
        .filter(|&id| {
            let n = tree.node(id);
            n.flags & skip == 0 && n.size >= opts.min_size.max(1) && !excluded[id as usize]
        })
        .collect();
    let mut report = DupReport { candidates: candidates.len() as u64, ..Default::default() };

    // 1. Por tamaño.
    candidates.sort_unstable_by_key(|&id| (tree.node(id).size, id));
    let by_size: Vec<Vec<NodeId>> = candidates
        .chunk_by(|&a, &b| tree.node(a).size == tree.node(b).size)
        .filter(|g| g.len() > 1)
        .map(<[NodeId]>::to_vec)
        .collect();

    // 2. Hash parcial. 3. Hash completo de los que siguen empatados.
    let partial =
        split_by_hash(tree, &pool, by_size, progress, &mut report, |path, size| partial_hash(path, size, progress));
    let (whole, sure): (Vec<_>, Vec<_>) = partial.into_iter().partition(|g| tree.node(g[0]).size > 2 * PART as u64);
    let mut groups = sure;
    groups.extend(split_by_hash(tree, &pool, whole, progress, &mut report, |path, _| full_hash(path, progress)));

    let (files, bytes) = progress.snapshot();
    report.read_files = files;
    report.read_bytes = bytes;
    report.groups = groups
        .into_iter()
        .map(|mut g| {
            g.sort_by_cached_key(|&id| tree.path(id));
            let n = tree.node(g[0]);
            DupGroup { size: n.size, alloc: if n.alloc > 0 { n.alloc } else { n.size }, files: g }
        })
        .collect();
    report.groups.sort_by(|a, b| b.wasted().cmp(&a.wasted()).then(b.size.cmp(&a.size)));
    report.elapsed = start.elapsed();
    Ok(report)
}

/// Divide cada grupo según el hash de sus archivos (en paralelo).
fn split_by_hash(
    tree: &Tree,
    pool: &rayon::ThreadPool,
    groups: Vec<Vec<NodeId>>,
    progress: &DupProgress,
    report: &mut DupReport,
    hash: impl Fn(&PathBuf, u64) -> io::Result<u128> + Sync,
) -> Vec<Vec<NodeId>> {
    let jobs: Vec<NodeId> = groups.iter().flatten().copied().collect();
    let hashes: Vec<Option<u128>> = pool.install(|| {
        jobs.par_iter()
            .map(|&id| {
                if progress.is_cancelled() {
                    return None;
                }
                hash(&tree.path(id), tree.node(id).size).ok()
            })
            .collect()
    });
    report.errors += hashes.iter().filter(|h| h.is_none()).count() as u64;

    let mut out = Vec::new();
    let mut i = 0;
    for g in groups {
        let mut keyed: Vec<(u128, NodeId)> =
            g.iter().zip(&hashes[i..i + g.len()]).filter_map(|(&id, h)| h.map(|h| (h, id))).collect();
        i += g.len();
        keyed.sort_unstable();
        for same in keyed.chunk_by(|a, b| a.0 == b.0) {
            if same.len() > 1 {
                out.push(same.iter().map(|&(_, id)| id).collect());
            }
        }
    }
    out
}

fn open(path: &PathBuf) -> io::Result<File> {
    let mut opts = File::options();
    opts.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_FLAG_SEQUENTIAL_SCAN: la caché del sistema lee por adelantado.
        opts.custom_flags(0x0800_0000);
    }
    opts.open(path)
}

/// Hash del tamaño, el principio y el final del archivo.
fn partial_hash(path: &PathBuf, size: u64, progress: &DupProgress) -> io::Result<u128> {
    let mut f = open(path)?;
    BUF.with_borrow_mut(|buf| {
        let mut h = Xxh3::new();
        h.update(&size.to_le_bytes());
        let head = (size as usize).min(PART);
        f.read_exact(&mut buf[..head])?;
        h.update(&buf[..head]);
        let mut read = head;
        if size > PART as u64 {
            let tail = ((size - PART as u64) as usize).min(PART);
            f.seek(SeekFrom::Start(size - tail as u64))?;
            f.read_exact(&mut buf[..tail])?;
            h.update(&buf[..tail]);
            read += tail;
        }
        progress.files.fetch_add(1, Relaxed);
        progress.bytes.fetch_add(read as u64, Relaxed);
        Ok(h.digest128())
    })
}

/// Hash de todo el contenido.
fn full_hash(path: &PathBuf, progress: &DupProgress) -> io::Result<u128> {
    let mut f = open(path)?;
    BUF.with_borrow_mut(|buf| {
        let mut h = Xxh3::new();
        loop {
            if progress.is_cancelled() {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelado"));
            }
            let n = f.read(buf)?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
            progress.bytes.fetch_add(n as u64, Relaxed);
        }
        progress.files.fetch_add(1, Relaxed);
        Ok(h.digest128())
    })
}

/// Marca las carpetas donde los duplicados son normales y no se deben tocar.
fn excluded_subtrees(tree: &Tree) -> Vec<bool> {
    let system: Vec<String> = ["SystemRoot", "ProgramFiles", "ProgramFiles(x86)", "ProgramW6432", "ProgramData"]
        .iter()
        .filter_map(std::env::var_os)
        .map(|v| v.to_string_lossy().trim_end_matches('\\').to_lowercase())
        .collect();
    let names = ["node_modules", ".git", "AppData", "$Recycle.Bin", "System Volume Information", "WindowsApps"];
    let mut excluded = vec![false; tree.len()];
    for id in 1..tree.len() as NodeId {
        let node = tree.node(id);
        if excluded[node.parent as usize] {
            excluded[id as usize] = true;
            continue;
        }
        if !node.is_dir() {
            continue;
        }
        let name = tree.name_bytes(id);
        let metafile = node.parent == ROOT && name.first() == Some(&b'$');
        let by_name = names.iter().any(|n| name.eq_ignore_ascii_case(n.as_bytes()));
        let by_path = node.parent == ROOT && system.contains(&tree.path(id).to_string_lossy().to_lowercase());
        excluded[id as usize] = metafile || by_name || by_path;
    }
    excluded
}
