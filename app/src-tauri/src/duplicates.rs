//! Sección «Duplicados»: archivos con el mismo contenido.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use itsuki_core::{DupOptions, DupProgress, find_duplicates};
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::state::{AppState, CANCELLED, ProgressEvent, run_blocking, with_progress};

/// Grupos que se envían a la interfaz como máximo (los de más espacio recuperable).
const MAX_GROUPS: usize = 300;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DupFile {
    name: String,
    folder: String,
    path: String,
    /// Última modificación en segundos Unix (0 = desconocida).
    modified: i64,
}

/// Copias idénticas, de la más antigua a la más reciente.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DupGroupView {
    size: u64,
    wasted: u64,
    files: Vec<DupFile>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DupView {
    groups: Vec<DupGroupView>,
    total_groups: usize,
    wasted: u64,
    read_files: u64,
    read_bytes: u64,
    /// Archivos que no se pudieron leer (en uso, sin permiso…).
    errors: u64,
    elapsed_ms: u64,
}

/// Carpetas personales: Escritorio, Documentos, Descargas, Imágenes, Vídeos,
/// Música y OneDrive (donde los duplicados son tuyos y no de programas).
fn personal_folders(profile: &Path) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> =
        ["Desktop", "Documents", "Downloads", "Pictures", "Videos", "Music"].iter().map(|n| profile.join(n)).collect();
    if let Ok(entries) = std::fs::read_dir(profile) {
        roots.extend(
            entries.flatten().filter(|e| e.file_name().to_string_lossy().starts_with("OneDrive")).map(|e| e.path()),
        );
    }
    roots
}

/// Busca duplicados de `min` bytes o más. `scope` es `"personal"` (carpetas
/// personales) o la raíz de una unidad. Se omiten Windows, programas, juegos,
/// AppData y carpetas ocultas.
#[tauri::command]
pub async fn find_dupes(app: AppHandle, scope: String, min: u64) -> Result<DupView, String> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let (tree, include_roots) = if scope == "personal" {
            let profile = std::env::var_os("USERPROFILE")
                .map(PathBuf::from)
                .ok_or_else(|| "no se encontró la carpeta del usuario".to_string())?;
            // Si ya se escaneó la unidad entera, se reutiliza.
            let tree = match state.cached_containing(&profile) {
                Some(t) => t,
                None => state.tree(&app, &profile, false)?,
            };
            (tree, personal_folders(&profile))
        } else {
            (state.tree(&app, Path::new(&scope), false)?, Vec::new())
        };

        let opts = DupOptions { min_size: min.max(1), include_system: false, include_roots, threads: 0 };
        let progress = Arc::new(DupProgress::new());
        state.set_dupes_job(Some(progress.clone()));
        let reader = progress.clone();
        let result = with_progress(
            &app,
            "dupes-progress",
            move || {
                let (files, bytes) = reader.snapshot();
                ProgressEvent { files, bytes }
            },
            || find_duplicates(&tree, &opts, &progress),
        );
        state.set_dupes_job(None);
        if progress.is_cancelled() {
            return Err(CANCELLED.into());
        }
        let report = result.map_err(|e| e.to_string())?;

        let groups = report
            .groups
            .iter()
            .take(MAX_GROUPS)
            .map(|g| {
                let mut files: Vec<DupFile> = g
                    .files
                    .iter()
                    .map(|&id| {
                        let path = tree.path(id);
                        DupFile {
                            name: tree.name_lossy(id).into_owned(),
                            folder: path.parent().map(|p| p.display().to_string()).unwrap_or_default(),
                            path: path.display().to_string(),
                            modified: tree.node(id).mtime,
                        }
                    })
                    .collect();
                files.sort_by(|a, b| a.modified.cmp(&b.modified).then_with(|| a.path.cmp(&b.path)));
                DupGroupView { size: g.size, wasted: g.wasted(), files }
            })
            .collect();
        Ok(DupView {
            groups,
            total_groups: report.groups.len(),
            wasted: report.wasted(),
            read_files: report.read_files,
            read_bytes: report.read_bytes,
            errors: report.errors,
            elapsed_ms: report.elapsed.as_millis() as u64,
        })
    })
    .await
}
