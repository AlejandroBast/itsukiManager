//! Motor portable basado en `std::fs`. Funciona en cualquier sistema y sirve
//! de referencia para comprobar el motor nativo.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use super::walk::{Backend, Batch};
use crate::tree::flags;

pub(crate) struct PortableBackend;

impl Backend for PortableBackend {
    type Dir = PathBuf;
    type Key = OsString;

    fn open_root(&self, path: &Path) -> io::Result<PathBuf> {
        Ok(path.to_path_buf())
    }

    fn open_child(&self, parent: &PathBuf, key: &OsString) -> io::Result<PathBuf> {
        Ok(parent.join(key))
    }

    fn list(&self, dir: &PathBuf, batch: &mut Batch<OsString>) -> io::Result<()> {
        for entry in fs::read_dir(dir)? {
            let Ok(entry) = entry else { continue };
            let Ok(kind) = entry.file_type() else { continue };
            let meta = entry.metadata().ok();
            let name = entry.file_name();

            let link = kind.is_symlink();
            let mut fl = 0u16;
            if kind.is_dir() {
                fl |= flags::DIR;
            }
            if link {
                fl |= flags::LINK | flags::REPARSE;
            }
            if let Some(m) = &meta {
                fl |= platform_flags(m, link);
            }

            let is_dir = fl & flags::DIR != 0;
            let (size, alloc, id) = match (&meta, is_dir) {
                (Some(m), false) => (m.len(), allocated(m), hardlink_id(m)),
                _ => (0, 0, 0),
            };
            let mtime = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs() as i64);

            let name_start = batch.names.len();
            batch.names.extend_from_slice(name.as_encoded_bytes());
            let key = (is_dir && !link).then_some(name);
            batch.push(name_start, size, alloc, mtime, id, fl, key);
        }
        Ok(())
    }
}

#[cfg(windows)]
fn platform_flags(m: &fs::Metadata, link: bool) -> u16 {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_COMPRESSED, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_OFFLINE,
        FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS, FILE_ATTRIBUTE_RECALL_ON_OPEN, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_ATTRIBUTE_SPARSE_FILE, FILE_ATTRIBUTE_SYSTEM,
    };
    let a = m.file_attributes();
    let mut fl = 0;
    if link && a & FILE_ATTRIBUTE_DIRECTORY != 0 {
        fl |= flags::DIR;
    }
    if a & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        fl |= flags::REPARSE;
    }
    if a & FILE_ATTRIBUTE_HIDDEN != 0 {
        fl |= flags::HIDDEN;
    }
    if a & FILE_ATTRIBUTE_SYSTEM != 0 {
        fl |= flags::SYSTEM;
    }
    if a & FILE_ATTRIBUTE_COMPRESSED != 0 {
        fl |= flags::COMPRESSED;
    }
    if a & FILE_ATTRIBUTE_SPARSE_FILE != 0 {
        fl |= flags::SPARSE;
    }
    let cloud = FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS | FILE_ATTRIBUTE_RECALL_ON_OPEN | FILE_ATTRIBUTE_OFFLINE;
    if a & FILE_ATTRIBUTE_DIRECTORY == 0 && a & cloud != 0 {
        fl |= flags::CLOUD;
    }
    fl
}

#[cfg(not(windows))]
fn platform_flags(_m: &fs::Metadata, _link: bool) -> u16 {
    0
}

/// Espacio en disco. En Windows `std` no lo expone: se usa el tamaño lógico.
#[cfg(unix)]
fn allocated(m: &fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    m.blocks() * 512
}

#[cfg(not(unix))]
fn allocated(m: &fs::Metadata) -> u64 {
    m.len()
}

/// Identificador para detectar enlaces duros (solo si el archivo tiene varios).
#[cfg(unix)]
fn hardlink_id(m: &fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    if m.nlink() > 1 { m.ino() } else { 0 }
}

#[cfg(not(unix))]
fn hardlink_id(_m: &fs::Metadata) -> u64 {
    0
}
