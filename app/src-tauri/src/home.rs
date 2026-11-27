//! Pantalla de inicio: unidad del sistema, reparto del espacio, carpetas más
//! grandes y lo que se puede liberar.

use itsuki_core::clean::{JunkContext, Safety, find_junk};
use itsuki_core::system::{self, DriveKind, Volume};
use itsuki_core::{Metric, ROOT, Tree, flags};
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::state::{AppState, lock, run_blocking, system_root};

/// Unidad tal como la muestran la tarjeta de almacenamiento y los selectores.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskInfo {
    pub root: String,
    pub name: String,
    pub kind: String,
    pub filesystem: String,
    pub total: u64,
    pub used: u64,
    pub free: u64,
}

/// Porción de la gráfica de uso.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSlice {
    pub key: &'static str,
    pub label: &'static str,
    pub bytes: u64,
}

/// Carpeta de la lista de inicio.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderRow {
    pub name: String,
    pub path: String,
    pub bytes: u64,
}

/// Todo lo que necesita la pantalla de inicio.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeSummary {
    pub root: String,
    pub used: u64,
    pub usage: Vec<UsageSlice>,
    pub folders: Vec<FolderRow>,
    pub reclaimable: u64,
    pub engine: String,
    pub elapsed_ms: u64,
}

pub fn disk_info(v: &Volume) -> DiskInfo {
    let letter = v.root.display().to_string().trim_end_matches('\\').to_string();
    let name = if v.label.trim().is_empty() {
        format!("Disco local ({letter})")
    } else {
        format!("{} ({letter})", v.label.trim())
    };
    let kind = match v.seek_penalty {
        Some(false) => "SSD",
        Some(true) => "HDD",
        None => "Disco",
    };
    DiskInfo {
        root: v.root.display().to_string(),
        name,
        kind: kind.into(),
        filesystem: v.filesystem.clone(),
        total: v.total,
        used: v.used(),
        free: v.free,
    }
}

/// Unidad del sistema para la tarjeta de almacenamiento.
#[tauri::command]
pub fn system_disk() -> Option<DiskInfo> {
    system::volume_of(&system_root()).map(|v| disk_info(&v))
}

/// Unidades locales y extraíbles que se pueden analizar.
#[tauri::command]
pub fn list_drives() -> Vec<DiskInfo> {
    system::volumes()
        .iter()
        .filter(|v| v.total > 0 && matches!(v.kind, DriveKind::Fixed | DriveKind::Removable))
        .map(disk_info)
        .collect()
}

/// Resumen de la pantalla de inicio. Con `refresh` vuelve a escanear.
#[tauri::command]
pub async fn home_summary(app: AppHandle, refresh: bool) -> Result<HomeSummary, String> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        if !refresh {
            if let Some(cached) = lock(&state.home).clone() {
                return Ok(cached);
            }
        }
        let tree = state.tree(&app, &system_root(), refresh)?;
        let summary = summarize(&tree);
        *lock(&state.home) = Some(summary.clone());
        Ok(summary)
    })
    .await
}

fn summarize(tree: &Tree) -> HomeSummary {
    let volume = system::volume_of(tree.root_path());
    let used = volume.as_ref().map_or(tree.node(ROOT).alloc, Volume::used);
    let junk = find_junk(tree, &JunkContext::from_env());
    let reclaimable = junk.items.iter().filter(|i| i.safety != Safety::Advice).map(|i| i.bytes).sum();
    let folders = tree
        .children_sorted(ROOT, Metric::Disk)
        .into_iter()
        .filter(|&id| tree.node(id).is_dir())
        .take(5)
        .map(|id| FolderRow {
            name: tree.name_lossy(id).into_owned(),
            path: tree.path(id).display().to_string(),
            bytes: tree.node(id).alloc,
        })
        .collect();

    HomeSummary {
        root: tree.root_path().display().to_string(),
        used,
        usage: usage_slices(tree, used),
        folders,
        reclaimable,
        engine: tree.stats().engine.to_string(),
        elapsed_ms: tree.stats().elapsed.as_millis() as u64,
    }
}

/// Carpetas de juegos conocidas (en cualquier nivel).
const GAME_DIRS: [&str; 14] = [
    "steamapps",
    "steamlibrary",
    "epic games",
    "riot games",
    "xboxgames",
    "gog games",
    "gog galaxy",
    "ea games",
    "origin games",
    "ubisoft game launcher",
    "battle.net",
    "rockstar games",
    "games",
    "juegos",
];

const APPS: usize = 0;
const FILES: usize = 1;
const GAMES: usize = 2;
const SYSTEM: usize = 3;
const OTHER: usize = 4;

/// Reparte el espacio usado en las cinco categorías de la gráfica según dónde
/// está cada archivo. Lo que el escaneo no puede ver (carpetas protegidas,
/// puntos de restauración sin permisos de administrador) cuenta como "Otros",
/// para que la gráfica sume lo mismo que la unidad.
fn usage_slices(tree: &Tree, used: u64) -> Vec<UsageSlice> {
    let at_drive_root = tree.root_path().parent().is_none();
    let mut category = vec![OTHER; tree.len()];
    let mut bytes = [0u64; 5];
    for id in 1..tree.len() as u32 {
        let node = tree.node(id);
        let parent = node.parent;
        let name = tree.name_lossy(id).to_lowercase();
        let inherited = category[parent as usize];
        let cat = if node.is_dir() && GAME_DIRS.contains(&name.as_str()) {
            GAMES
        } else if node.is_dir() && name == "appdata" && inherited == FILES {
            APPS
        } else if parent == ROOT && at_drive_root {
            match name.as_str() {
                "windows"
                | "system volume information"
                | "recovery"
                | "boot"
                | "$windows.~bt"
                | "$windows.~ws"
                | "$winreagent"
                | "pagefile.sys"
                | "hiberfil.sys"
                | "swapfile.sys"
                | "dumpstack.log.tmp"
                | "bootmgr" => SYSTEM,
                "program files" | "program files (x86)" | "programdata" => APPS,
                "users" => FILES,
                "$recycle.bin" => OTHER,
                n if n.starts_with('$') => SYSTEM,
                _ => OTHER,
            }
        } else {
            inherited
        };
        category[id as usize] = cat;
        if !node.is_dir() && !node.has(flags::HARDLINK) {
            bytes[cat] += node.alloc;
        }
    }
    let counted: u64 = bytes.iter().sum();
    bytes[OTHER] += used.saturating_sub(counted);

    let labels = [
        ("apps", "Aplicaciones"),
        ("files", "Archivos"),
        ("games", "Juegos"),
        ("system", "Sistema"),
        ("other", "Otros"),
    ];
    labels.iter().zip(bytes).map(|(&(key, label), bytes)| UsageSlice { key, label, bytes }).collect()
}
