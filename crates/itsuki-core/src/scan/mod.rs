//! Escaneo del disco.
//!
//! ```no_run
//! use itsuki_core::scan::{scan, ScanOptions, ScanProgress};
//!
//! let progress = ScanProgress::new();
//! let tree = scan("C:\\", &ScanOptions::default(), &progress).unwrap();
//! println!("{} bytes en disco", tree.node(tree.root()).alloc);
//! ```

mod mft;
#[cfg(windows)]
mod native;
mod portable;
mod util;
mod walk;

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::system;
use crate::tree::{ScanStats, Tree};

/// Motor de lectura de carpetas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Engine {
    /// El más rápido disponible: MFT si se escanea una unidad NTFS completa
    /// como administrador; si no, el nativo.
    #[default]
    Auto,
    /// Lectura directa de la MFT de NTFS (administrador, volúmenes locales).
    Mft,
    /// API nativa de Windows (NtCreateFile + GetFileInformationByHandleEx).
    Native,
    /// `std::fs`, portable y más lento.
    Portable,
}

/// Opciones de escaneo.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScanOptions {
    /// Hilos de trabajo (0 = automático).
    pub threads: usize,
    /// Contar una sola vez los archivos con varios enlaces duros.
    pub dedupe_hardlinks: bool,
    pub engine: Engine,
}

impl Default for ScanOptions {
    fn default() -> Self {
        ScanOptions { threads: 0, dedupe_hardlinks: true, engine: Engine::Auto }
    }
}

/// Progreso en vivo de un escaneo. Se puede leer desde otro hilo mientras se
/// escanea, y permite cancelarlo.
#[derive(Debug, Default)]
pub struct ScanProgress {
    files: AtomicU64,
    dirs: AtomicU64,
    bytes: AtomicU64,
    errors: AtomicU64,
    cancel: AtomicBool,
}

/// Foto del progreso.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Progress {
    pub files: u64,
    pub dirs: u64,
    /// Bytes en disco encontrados hasta ahora.
    pub bytes: u64,
    pub errors: u64,
}

impl ScanProgress {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot(&self) -> Progress {
        Progress {
            files: self.files.load(Relaxed),
            dirs: self.dirs.load(Relaxed),
            bytes: self.bytes.load(Relaxed),
            errors: self.errors.load(Relaxed),
        }
    }

    /// Pide detener el escaneo lo antes posible.
    pub fn cancel(&self) {
        self.cancel.store(true, Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Relaxed)
    }

    pub(crate) fn add(&self, files: u64, dirs: u64, bytes: u64) {
        self.files.fetch_add(files, Relaxed);
        self.dirs.fetch_add(dirs, Relaxed);
        self.bytes.fetch_add(bytes, Relaxed);
    }

    /// Vuelve a cero los contadores (no la cancelación).
    pub(crate) fn reset(&self) {
        for c in [&self.files, &self.dirs, &self.bytes, &self.errors] {
            c.store(0, Relaxed);
        }
    }
}

/// Error de escaneo.
#[derive(Debug)]
pub enum ScanError {
    /// No se pudo abrir la ruta.
    Io {
        path: PathBuf,
        source: io::Error,
    },
    /// La ruta no es una carpeta.
    NotADirectory(PathBuf),
    /// Se canceló con [`ScanProgress::cancel`].
    Cancelled,
    /// El motor pedido no existe en este sistema.
    Unsupported(&'static str),
    /// El motor MFT necesita permisos de administrador.
    NeedsAdmin,
    ThreadPool(String),
}

impl fmt::Display for ScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScanError::Io { path, source } => write!(f, "no se pudo leer {}: {source}", path.display()),
            ScanError::NotADirectory(p) => write!(f, "{} no es una carpeta", p.display()),
            ScanError::Cancelled => write!(f, "escaneo cancelado"),
            ScanError::Unsupported(what) => write!(f, "no disponible en este sistema: {what}"),
            ScanError::NeedsAdmin => write!(f, "el motor MFT necesita ejecutar itsuki como administrador"),
            ScanError::ThreadPool(e) => write!(f, "no se pudieron crear los hilos: {e}"),
        }
    }
}

impl std::error::Error for ScanError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ScanError::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Hilos por defecto: todos los núcleos lógicos (mínimo 4, porque el escaneo
/// pasa buena parte del tiempo esperando al sistema).
pub fn default_threads() -> usize {
    std::thread::available_parallelism().map_or(4, |n| n.get()).max(4)
}

/// Escanea `root` y devuelve el árbol completo con los tamaños calculados.
pub fn scan(root: impl AsRef<Path>, opts: &ScanOptions, progress: &ScanProgress) -> Result<Tree, ScanError> {
    let start = Instant::now();
    let root = normalize_root(root.as_ref())?;
    let threads = if opts.threads == 0 { default_threads() } else { opts.threads };
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .thread_name(|i| format!("itsuki-escaneo-{i}"))
        .build()
        .map_err(|e| ScanError::ThreadPool(e.to_string()))?;

    let elevated = system::is_elevated();
    let backup_privilege = elevated && system::enable_backup_privilege();
    let io_err = |source| ScanError::Io { path: root.clone(), source };

    let keep_ids = opts.dedupe_hardlinks;
    let mut mft_error = None;
    let (engine, out) = match opts.engine {
        Engine::Portable => {
            ("portable", walk::run(&portable::PortableBackend, &root, &pool, progress, keep_ids).map_err(io_err)?)
        }
        #[cfg(windows)]
        Engine::Mft => {
            if !elevated {
                return Err(ScanError::NeedsAdmin);
            }
            ("mft", mft::scan(&root, &pool, progress, keep_ids).map_err(io_err)?)
        }
        #[cfg(windows)]
        Engine::Auto if elevated && root.parent().is_none() => match mft::scan(&root, &pool, progress, keep_ids) {
            Ok(out) => ("mft", out),
            Err(e) => {
                // Volumen no NTFS, de red, etc.: se usa el motor nativo.
                mft_error = Some(e.to_string());
                progress.reset();
                ("nativo", walk::run(&native::NativeBackend::new(), &root, &pool, progress, keep_ids).map_err(io_err)?)
            }
        },
        #[cfg(windows)]
        Engine::Auto | Engine::Native => {
            ("nativo", walk::run(&native::NativeBackend::new(), &root, &pool, progress, keep_ids).map_err(io_err)?)
        }
        #[cfg(not(windows))]
        Engine::Mft => return Err(ScanError::Unsupported("motor MFT de Windows")),
        #[cfg(not(windows))]
        Engine::Native => return Err(ScanError::Unsupported("motor nativo de Windows")),
        #[cfg(not(windows))]
        Engine::Auto => {
            ("portable", walk::run(&portable::PortableBackend, &root, &pool, progress, keep_ids).map_err(io_err)?)
        }
    };
    if progress.is_cancelled() {
        return Err(ScanError::Cancelled);
    }
    let stats = ScanStats { engine, threads, elevated, backup_privilege, mft_error, ..Default::default() };
    let mut tree = Tree::build(out.nodes, out.names, out.ids, root, out.denied, stats);
    tree.set_elapsed(start.elapsed());
    Ok(tree)
}

/// Ruta absoluta y limpia. `C:` se interpreta como la raíz de la unidad.
fn normalize_root(path: &Path) -> Result<PathBuf, ScanError> {
    let mut raw = path.as_os_str().to_owned();
    let bytes = raw.as_encoded_bytes();
    if cfg!(windows) && bytes.len() == 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        raw.push("\\");
    }
    let io_err = |source| ScanError::Io { path: path.to_path_buf(), source };
    let abs: PathBuf = std::path::absolute(&raw).map_err(io_err)?.components().collect();
    let meta = std::fs::metadata(&abs).map_err(io_err)?;
    if !meta.is_dir() {
        return Err(ScanError::NotADirectory(abs));
    }
    Ok(abs)
}
