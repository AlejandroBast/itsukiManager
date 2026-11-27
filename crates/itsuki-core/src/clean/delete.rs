//! Borrado de la basura detectada.
//!
//! Reglas de seguridad:
//! - Nunca sigue enlaces simbólicos ni uniones: borra el enlace, no el destino.
//! - No toca otros puntos de reparse (por ejemplo, archivos de OneDrive que
//!   solo están en la nube): borrarlos los borraría también en la nube.
//! - Lo que está en uso o sin permiso se salta y se cuenta.
//! - Solo actúa sobre rutas permitidas: nada protegido, salvo las carpetas de
//!   basura conocidas de Windows (Temp, descargas de Windows Update, volcados).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::Serialize;

use super::{Action, JunkItem, path_starts_with};
use crate::actions::{protection_reason, send_to_recycle_bin};
use crate::tree::Tree;

/// Resultado de una limpieza.
#[derive(Clone, Debug, Default, Serialize)]
pub struct DeleteReport {
    pub files: u64,
    /// Bytes de datos liberados.
    pub bytes: u64,
    /// Elementos que no se pudieron borrar (en uso, sin permiso…).
    pub skipped: u64,
    /// Primeros errores, para mostrarlos.
    pub errors: Vec<String>,
}

impl DeleteReport {
    pub fn merge(&mut self, other: DeleteReport) {
        self.files += other.files;
        self.bytes += other.bytes;
        self.skipped += other.skipped;
        let room = 20usize.saturating_sub(self.errors.len());
        self.errors.extend(other.errors.into_iter().take(room));
    }

    fn fail(&mut self, path: &Path, e: &io::Error) {
        self.skipped += 1;
        if self.errors.len() < 20 {
            self.errors.push(format!("{}: {e}", path.display()));
        }
    }
}

/// Carpetas de basura dentro de Windows que sí se pueden vaciar.
const WINDOWS_JUNK: [&str; 6] = [
    "Temp",
    "SoftwareDistribution\\Download",
    "Minidump",
    "LiveKernelReports",
    "MEMORY.DMP",
    "ServiceProfiles\\NetworkService\\AppData\\Local\\Microsoft\\Windows\\DeliveryOptimization\\Cache",
];

/// `true` si se permite borrar `path` como parte de una limpieza.
pub fn junk_path_allowed(path: &Path) -> bool {
    if let Some(win) = std::env::var_os("SystemRoot").map(PathBuf::from) {
        if WINDOWS_JUNK.iter().any(|rel| path_starts_with(path, &win.join(rel))) {
            return true;
        }
    }
    protection_reason(path).is_none()
}

fn ensure_allowed(path: &Path) -> io::Result<()> {
    if junk_path_allowed(path) {
        Ok(())
    } else {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("{} está protegida", path.display())))
    }
}

/// Limpia un hallazgo. Con `to_recycle_bin` lo envía a la Papelera en vez de
/// borrarlo de forma permanente.
pub fn clean_item(tree: &Tree, item: &JunkItem, to_recycle_bin: bool) -> io::Result<DeleteReport> {
    let mut report = DeleteReport::default();
    let Some(&first) = item.nodes.first() else { return Ok(report) };
    match &item.action {
        Action::Advice(_) => {
            return Err(io::Error::new(io::ErrorKind::Unsupported, "es una sugerencia: no se borra desde aquí"));
        }
        Action::EmptyRecycleBin => {
            empty_recycle_bin(&tree.path(first))?;
            report.files = item.files;
            report.bytes = item.bytes;
        }
        Action::Contents { min_age } => {
            let dir = tree.path(first);
            ensure_allowed(&dir)?;
            let cutoff = SystemTime::now() - Duration::from_secs((*min_age).max(0) as u64);
            let entries: Vec<PathBuf> = fs::read_dir(&dir)?
                .filter_map(Result::ok)
                .filter(|e| *min_age <= 0 || e.metadata().and_then(|m| m.modified()).map_or(true, |t| t <= cutoff))
                .map(|e| e.path())
                .collect();
            dispose(&entries, item, to_recycle_bin, &mut report);
        }
        Action::Folder => {
            let path = tree.path(first);
            ensure_allowed(&path)?;
            dispose(&[path], item, to_recycle_bin, &mut report);
        }
        Action::Files => {
            let paths: Vec<PathBuf> = item.nodes.iter().map(|&id| tree.path(id)).collect();
            for p in &paths {
                ensure_allowed(p)?;
            }
            dispose(&paths, item, to_recycle_bin, &mut report);
        }
    }
    Ok(report)
}

/// Borra (o envía a la Papelera) una lista de rutas.
fn dispose(paths: &[PathBuf], item: &JunkItem, to_recycle_bin: bool, report: &mut DeleteReport) {
    if paths.is_empty() {
        return;
    }
    if to_recycle_bin {
        match send_to_recycle_bin(paths) {
            Ok(()) => {
                report.files += item.files;
                report.bytes += item.bytes;
            }
            Err(e) => report.fail(&paths[0], &e),
        }
        return;
    }
    for p in paths {
        remove_tree(p, report);
    }
}

/// Borra `path` y su contenido sin seguir enlaces. Sigue con lo demás si
/// algo falla.
#[allow(clippy::permissions_set_readonly_false)]
pub(crate) fn remove_tree(path: &Path, report: &mut DeleteReport) {
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) => return report.fail(path, &e),
    };
    let kind = meta.file_type();
    if kind.is_symlink() {
        // Enlace simbólico o unión: se borra el enlace, nunca lo que hay detrás.
        let result = if link_is_dir(&meta) { fs::remove_dir(path) } else { fs::remove_file(path) };
        if let Err(e) = result {
            report.fail(path, &e);
        }
        return;
    }
    if other_reparse_point(&meta) {
        return report
            .fail(path, &io::Error::other("punto de reparse (por ejemplo, un archivo en la nube): no se toca"));
    }
    if kind.is_dir() {
        match fs::read_dir(path) {
            Ok(entries) => {
                for e in entries.flatten() {
                    remove_tree(&e.path(), report);
                }
            }
            Err(e) => return report.fail(path, &e),
        }
        // Si quedó algo en uso, la carpeta no está vacía y se conserva.
        let _ = fs::remove_dir(path);
        return;
    }
    let len = meta.len();
    let mut result = fs::remove_file(path);
    if result.as_ref().is_err_and(|e| e.kind() == io::ErrorKind::PermissionDenied) && meta.permissions().readonly() {
        let mut perms = meta.permissions();
        perms.set_readonly(false);
        let _ = fs::set_permissions(path, perms);
        result = fs::remove_file(path);
    }
    match result {
        Ok(()) => {
            report.files += 1;
            report.bytes += len;
        }
        Err(e) => report.fail(path, &e),
    }
}

#[cfg(windows)]
fn link_is_dir(meta: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    meta.file_attributes() & 0x10 != 0
}

#[cfg(not(windows))]
fn link_is_dir(_meta: &fs::Metadata) -> bool {
    false
}

#[cfg(windows)]
fn other_reparse_point(meta: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    meta.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn other_reparse_point(_meta: &fs::Metadata) -> bool {
    false
}

/// Vacía la Papelera de la unidad de `bin` (`X:\$Recycle.Bin`).
#[cfg(windows)]
fn empty_recycle_bin(bin: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::{SHERB_NOCONFIRMATION, SHERB_NOPROGRESSUI, SHERB_NOSOUND, SHEmptyRecycleBinW};

    let root = bin.parent().unwrap_or(bin);
    let wide: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: `wide` termina en NUL.
    let hr = unsafe {
        SHEmptyRecycleBinW(
            std::ptr::null_mut(),
            wide.as_ptr(),
            SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND,
        )
    };
    // E_UNEXPECTED: la Papelera ya estaba vacía.
    if hr >= 0 || hr as u32 == 0x8000_FFFF {
        Ok(())
    } else {
        Err(io::Error::other(format!("no se pudo vaciar la Papelera (0x{:08X})", hr as u32)))
    }
}

#[cfg(not(windows))]
fn empty_recycle_bin(_bin: &Path) -> io::Result<()> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "la Papelera solo está disponible en Windows"))
}
