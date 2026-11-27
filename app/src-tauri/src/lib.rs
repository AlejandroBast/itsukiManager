//! Capa de presentación de escritorio de itsukiManager.
//!
//! Solo adapta los datos del núcleo (`itsuki-core`) a lo que muestra la
//! interfaz: no contiene lógica de escaneo ni de limpieza propia.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use itsuki_core::clean::{JunkContext, Safety, find_junk};
use itsuki_core::system::{self, Volume};
use itsuki_core::{Metric, ROOT, ScanOptions, ScanProgress, Tree, flags, scan};
use serde::Serialize;
use tauri::{Emitter, LogicalSize, Manager};

/// Unidad tal como la muestra la tarjeta de almacenamiento.
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

/// Fila de "Carpetas que más espacio ocupan".
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

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScanProgressEvent {
    files: u64,
    bytes: u64,
}

/// Último resumen calculado (evita volver a escanear al recargar la vista).
#[derive(Default)]
struct HomeCache(Mutex<Option<HomeSummary>>);

/// Raíz de la unidad del sistema (normalmente `C:\`).
fn system_root() -> PathBuf {
    let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
    PathBuf::from(format!("{drive}\\"))
}

fn disk_info(v: &Volume) -> DiskInfo {
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
fn system_disk() -> Option<DiskInfo> {
    system::volume_of(&system_root()).map(|v| disk_info(&v))
}

/// Resumen de la pantalla de inicio. Con `refresh` vuelve a escanear.
#[tauri::command]
async fn home_summary(
    app: tauri::AppHandle,
    cache: tauri::State<'_, HomeCache>,
    refresh: bool,
) -> Result<HomeSummary, String> {
    if !refresh {
        if let Some(cached) = cache.0.lock().map_err(|e| e.to_string())?.clone() {
            return Ok(cached);
        }
    }
    let summary =
        tauri::async_runtime::spawn_blocking(move || compute_home(&app)).await.map_err(|e| e.to_string())??;
    *cache.0.lock().map_err(|e| e.to_string())? = Some(summary.clone());
    Ok(summary)
}

/// Escanea la unidad del sistema enviando el progreso a la interfaz.
fn compute_home(app: &tauri::AppHandle) -> Result<HomeSummary, String> {
    let root = system_root();
    let progress = Arc::new(ScanProgress::new());
    let done = Arc::new(AtomicBool::new(false));
    let ticker = {
        let (app, progress, done) = (app.clone(), progress.clone(), done.clone());
        std::thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                let p = progress.snapshot();
                let _ = app.emit("scan-progress", ScanProgressEvent { files: p.files, bytes: p.bytes });
                std::thread::sleep(Duration::from_millis(150));
            }
        })
    };
    let result = scan(&root, &ScanOptions::default(), &progress);
    done.store(true, Ordering::Relaxed);
    let _ = ticker.join();
    let tree = result.map_err(|e| e.to_string())?;

    let volume = system::volume_of(&root);
    let used = volume.as_ref().map_or(tree.node(ROOT).alloc, Volume::used);
    let junk = find_junk(&tree, &JunkContext::from_env());
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

    Ok(HomeSummary {
        root: tree.root_path().display().to_string(),
        used,
        usage: usage_slices(&tree, used),
        folders,
        reclaimable,
        engine: tree.stats().engine.to_string(),
        elapsed_ms: tree.stats().elapsed.as_millis() as u64,
    })
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

/// La ventana tiene tamaño fijo (1080 × 720, en `tauri.conf.json`). Solo se reduce, sin perder la
/// proporción, si no cabe en el área útil de la pantalla (portátiles con escalado alto).
fn fit_small_screen(window: &tauri::WebviewWindow) {
    let (Ok(Some(monitor)), Ok(size), Ok(factor)) =
        (window.current_monitor(), window.inner_size(), window.scale_factor())
    else {
        return;
    };
    let size = size.to_logical::<f64>(factor);
    let area = monitor.work_area().size.to_logical::<f64>(monitor.scale_factor());
    let scale = (area.width * 0.96 / size.width).min(area.height * 0.96 / size.height);
    if scale < 1.0 {
        let _ = window.set_size(LogicalSize::new(size.width * scale, size.height * scale));
        let _ = window.center();
    }
}

pub fn run() {
    tauri::Builder::default()
        .manage(HomeCache::default())
        .invoke_handler(tauri::generate_handler![system_disk, home_summary])
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                fit_small_screen(&window);
                let _ = window.show();
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("no se pudo iniciar ItsukiManager");
}
