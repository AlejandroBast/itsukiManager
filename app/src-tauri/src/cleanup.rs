//! Sección «Limpieza»: busca basura conocida en la unidad del sistema y limpia
//! lo que el usuario elige, con las mismas protecciones que la línea de comandos.

use std::cmp::Reverse;
use std::collections::HashSet;
use std::path::{Component, Path};
use std::sync::Arc;

use itsuki_core::clean::apps::{app_owner, running_apps};
use itsuki_core::clean::{Action, DeleteReport, JunkCategory, JunkItem, JunkReport, Safety, clean_item, item_path};
use itsuki_core::{JunkContext, Tree, find_junk, system};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::state::{AppState, lock, run_blocking, system_root};

/// Último análisis: la limpieza actúa sobre estos hallazgos (por su posición).
pub struct JunkState {
    tree: Arc<Tree>,
    report: JunkReport,
}

fn safety_id(s: Safety) -> &'static str {
    match s {
        Safety::Safe => "safe",
        Safety::Regenerable => "regenerable",
        Safety::Review => "review",
        Safety::Advice => "advice",
    }
}

/// Nombre de la app tal como aparece en la ruta (`BraveSoftware`, `Claude`), para mostrarlo.
fn owner_label(path: &Path, owner: &str) -> String {
    path.components()
        .filter_map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .map(|c| match c.split_once('_') {
            Some((head, _)) if !head.is_empty() => head,
            _ => c,
        })
        .find(|c| c.to_lowercase() == owner)
        .map_or_else(|| owner.to_string(), str::to_string)
}

/// Aplicación abierta dueña de la caché: borrarla en uso puede dejarla en mal
/// estado, así que por defecto se salta.
fn open_owner(tree: &Tree, item: &JunkItem, open: &HashSet<String>) -> Option<String> {
    if !matches!(item.category, JunkCategory::BrowserCache | JunkCategory::AppCache | JunkCategory::GameCache) {
        return None;
    }
    let path = item_path(tree, item)?;
    let owner = app_owner(&path)?;
    open.contains(&owner).then(|| owner_label(&path, &owner))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JunkItemView {
    id: usize,
    label: &'static str,
    path: String,
    /// Ubicaciones adicionales del mismo hallazgo.
    more: usize,
    bytes: u64,
    files: u64,
    safety: &'static str,
    needs_admin: bool,
    open_app: Option<String>,
    /// Texto de la recomendación (solo en las sugerencias).
    advice: Option<&'static str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JunkCategoryView {
    id: &'static str,
    label: &'static str,
    safety: &'static str,
    bytes: u64,
    files: u64,
    items: Vec<JunkItemView>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JunkView {
    root: String,
    categories: Vec<JunkCategoryView>,
    safe: u64,
    regenerable: u64,
    review: u64,
    /// La app tiene permisos de administrador (sin ellos se saltan algunas ubicaciones).
    elevated: bool,
}

/// Busca basura en la unidad del sistema.
#[tauri::command]
pub async fn junk_scan(app: AppHandle, refresh: bool) -> Result<JunkView, String> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let tree = state.tree(&app, &system_root(), refresh)?;
        let report = find_junk(&tree, &JunkContext::from_env());
        let open = running_apps();
        let categories = report
            .by_category()
            .into_iter()
            .map(|c| {
                let mut items: Vec<JunkItemView> = report
                    .items
                    .iter()
                    .enumerate()
                    .filter(|(_, i)| i.category == c.category)
                    .map(|(id, i)| JunkItemView {
                        id,
                        label: i.label,
                        path: item_path(&tree, i).map(|p| p.display().to_string()).unwrap_or_default(),
                        more: i.nodes.len().saturating_sub(1),
                        bytes: i.bytes,
                        files: i.files,
                        safety: safety_id(i.safety),
                        needs_admin: i.needs_admin,
                        open_app: open_owner(&tree, i, &open),
                        advice: match &i.action {
                            Action::Advice(text) => Some(*text),
                            _ => None,
                        },
                    })
                    .collect();
                items.sort_by_key(|i| Reverse(i.bytes));
                JunkCategoryView {
                    id: c.category.id(),
                    label: c.category.label(),
                    safety: safety_id(c.safety),
                    bytes: c.bytes,
                    files: c.files,
                    items,
                }
            })
            .collect();
        let view = JunkView {
            root: tree.root_path().display().to_string(),
            categories,
            safe: report.total(Safety::Safe),
            regenerable: report.total(Safety::Regenerable),
            review: report.total(Safety::Review),
            elevated: system::is_elevated(),
        };
        *lock(&state.junk) = Some(JunkState { tree, report });
        Ok(view)
    })
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenSkip {
    app: String,
    bytes: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanResult {
    bytes: u64,
    files: u64,
    /// Elementos que no se pudieron borrar (en uso o sin permiso).
    skipped: u64,
    errors: Vec<String>,
    /// Cachés que se saltaron por ser de aplicaciones abiertas.
    skipped_open: Vec<OpenSkip>,
    to_recycle_bin: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CleanProgress {
    done: usize,
    total: usize,
    bytes: u64,
}

/// Limpia los hallazgos `ids` del último análisis. Con `to_recycle_bin` los
/// envía a la Papelera en vez de borrarlos; las cachés de aplicaciones abiertas
/// se saltan salvo con `include_open`.
#[tauri::command]
pub async fn junk_clean(
    app: AppHandle,
    ids: Vec<usize>,
    to_recycle_bin: bool,
    include_open: bool,
) -> Result<CleanResult, String> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let (tree, items) = {
            let junk = lock(&state.junk);
            let junk = junk.as_ref().ok_or_else(|| "vuelve a analizar antes de limpiar".to_string())?;
            let items: Vec<JunkItem> = ids
                .iter()
                .filter_map(|&i| junk.report.items.get(i))
                .filter(|i| i.safety != Safety::Advice)
                .cloned()
                .collect();
            (junk.tree.clone(), items)
        };

        let open = if include_open { HashSet::new() } else { running_apps() };
        let mut skipped_open: Vec<OpenSkip> = Vec::new();
        let mut selected = Vec::new();
        for item in items {
            match open_owner(&tree, &item, &open) {
                Some(owner) => match skipped_open.iter_mut().find(|s| s.app == owner) {
                    Some(s) => s.bytes += item.bytes,
                    None => skipped_open.push(OpenSkip { app: owner, bytes: item.bytes }),
                },
                None => selected.push(item),
            }
        }
        skipped_open.sort_by_key(|s| Reverse(s.bytes));

        let mut done = DeleteReport::default();
        let mut failed = Vec::new();
        let total = selected.len();
        for (n, item) in selected.iter().enumerate() {
            let _ = app.emit("clean-progress", CleanProgress { done: n, total, bytes: done.bytes });
            match clean_item(&tree, item, to_recycle_bin) {
                Ok(r) => done.merge(r),
                Err(e) => failed.push(format!("{}: {e}", item.label)),
            }
        }
        let _ = app.emit("clean-progress", CleanProgress { done: total, total, bytes: done.bytes });
        // Lo escaneado ya no refleja el disco.
        state.invalidate(&system_root());

        let skipped = done.skipped + failed.len() as u64;
        let mut errors = failed;
        errors.extend(done.errors);
        errors.truncate(20);
        Ok(CleanResult { bytes: done.bytes, files: done.files, skipped, errors, skipped_open, to_recycle_bin })
    })
    .await
}
