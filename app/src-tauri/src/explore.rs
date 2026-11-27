//! Secciones «Analizar disco» y «Archivos grandes», y las acciones sobre lo que
//! listan: mostrar en el Explorador y enviar a la Papelera.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use itsuki_core::actions::{reveal_in_explorer, send_to_recycle_bin};
use itsuki_core::system;
use itsuki_core::types::Category;
use itsuki_core::{DirMode, DirQuery, FileQuery, Metric, ROOT, Tree, flags};
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::state::{AppState, key_of, run_blocking};

/// Carpeta grande. `parent` es la carpeta grande que la contiene (0 si está en la raíz).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderItem {
    id: u32,
    parent: u32,
    name: String,
    path: String,
    bytes: u64,
    files: u64,
}

/// Datos del escaneo que se muestran junto a los resultados.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanInfo {
    root: String,
    /// Espacio usado de la unidad (lo escaneado, si no es una unidad entera).
    used: u64,
    /// Espacio de todo lo que se pudo leer (la diferencia con `used` queda sin contabilizar).
    scanned: u64,
    total: Option<u64>,
    files: u64,
    /// Carpetas que no se pudieron leer (sin permisos de administrador).
    denied: u64,
    engine: String,
    elapsed_ms: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FoldersView {
    scan: ScanInfo,
    folders: Vec<FolderItem>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileItem {
    name: String,
    folder: String,
    path: String,
    bytes: u64,
    /// Última modificación en segundos Unix (0 = desconocida).
    modified: i64,
    kind: &'static str,
    /// Oculto o de sistema.
    hidden: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilesView {
    scan: ScanInfo,
    files: Vec<FileItem>,
}

fn scan_info(tree: &Tree) -> ScanInfo {
    let volume = system::volume_of(tree.root_path()).filter(|v| key_of(&v.root) == key_of(tree.root_path()));
    let s = tree.stats();
    ScanInfo {
        root: tree.root_path().display().to_string(),
        used: volume.as_ref().map_or(tree.node(ROOT).alloc, |v| v.used()),
        scanned: tree.node(ROOT).alloc,
        total: volume.map(|v| v.total),
        files: s.files,
        denied: s.denied,
        engine: s.engine.to_string(),
        elapsed_ms: s.elapsed.as_millis() as u64,
    }
}

/// Carpetas de `min` bytes o más, con su jerarquía, de mayor a menor.
#[tauri::command]
pub async fn large_folders(app: AppHandle, root: String, min: u64, refresh: bool) -> Result<FoldersView, String> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let tree = state.tree(&app, Path::new(&root), refresh)?;
        let query = DirQuery { min: min.max(1), max_depth: None, mode: DirMode::All, metric: Metric::Disk };
        let folders = tree
            .large_dirs(&query)
            .iter()
            .map(|h| {
                let n = tree.node(h.id);
                FolderItem {
                    id: h.id,
                    parent: tree.parent(h.id).unwrap_or(ROOT),
                    name: tree.name_lossy(h.id).into_owned(),
                    path: tree.path(h.id).display().to_string(),
                    bytes: n.alloc,
                    files: u64::from(n.files),
                }
            })
            .collect();
        Ok(FoldersView { scan: scan_info(&tree), folders })
    })
    .await
}

/// Los `top` archivos que más espacio ocupan.
#[tauri::command]
pub async fn large_files(app: AppHandle, root: String, top: usize, refresh: bool) -> Result<FilesView, String> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let tree = state.tree(&app, Path::new(&root), refresh)?;
        let query = FileQuery { top: top.clamp(1, 1000), min: 0, modified_before: None, metric: Metric::Disk };
        let files = tree
            .top_files(&query)
            .iter()
            .map(|&id| {
                let n = tree.node(id);
                let path = tree.path(id);
                FileItem {
                    name: tree.name_lossy(id).into_owned(),
                    folder: path.parent().map(|p| p.display().to_string()).unwrap_or_default(),
                    path: path.display().to_string(),
                    bytes: n.alloc,
                    modified: n.mtime,
                    kind: tree.extension(id).map_or(Category::NoExtension, |e| Category::of_extension(&e)).label(),
                    hidden: n.has(flags::HIDDEN) || n.has(flags::SYSTEM),
                }
            })
            .collect();
        Ok(FilesView { scan: scan_info(&tree), files })
    })
    .await
}

/// Abre el Explorador con el elemento seleccionado.
#[tauri::command]
pub fn reveal_path(path: String) -> Result<(), String> {
    reveal_in_explorer(Path::new(&path)).map_err(|e| e.to_string())
}

/// Envía archivos o carpetas a la Papelera (las rutas protegidas se rechazan).
#[tauri::command]
pub async fn recycle_paths(app: AppHandle, paths: Vec<String>) -> Result<(), String> {
    run_blocking(move || {
        let list: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
        let result = send_to_recycle_bin(&list);
        // Las rutas protegidas o inexistentes se rechazan antes de tocar nada; en los
        // demás casos (aunque falle a medias) lo escaneado ya no refleja el disco.
        let untouched =
            matches!(&result, Err(e) if matches!(e.kind(), ErrorKind::PermissionDenied | ErrorKind::NotFound));
        if !untouched {
            let state = app.state::<AppState>();
            for p in &list {
                state.invalidate(p);
            }
        }
        result.map_err(|e| e.to_string())
    })
    .await
}
