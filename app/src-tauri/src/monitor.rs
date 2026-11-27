//! Vigilancia en segundo plano mientras la app está abierta (también desde la
//! bandeja): una foto diaria del historial de la unidad del sistema y una
//! notificación cuando una unidad pasa a «Aviso» o «Crítico».

use std::collections::HashMap;
use std::time::Duration;

use itsuki_core::alerts::{AlertRules, Level, volume_status};
use itsuki_core::format::now_unix;
use itsuki_core::history::{DEFAULT_MIN_FOLDER, History};
use itsuki_core::system::{self, DriveKind};
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::state::{AppState, system_root};

/// Deja terminar el análisis con el que arranca el inicio.
const FIRST_CHECK: Duration = Duration::from_secs(120);
const CHECK_EVERY: Duration = Duration::from_secs(3600);
/// Una foto al día (con margen para que la hora de la revisión no la retrase).
const SNAPSHOT_EVERY: i64 = 23 * 3600;

pub fn start(app: AppHandle) {
    std::thread::spawn(move || {
        // Nivel anterior de cada unidad: solo se avisa cuando empeora.
        let mut levels: HashMap<String, Level> = HashMap::new();
        std::thread::sleep(FIRST_CHECK);
        loop {
            if let Err(e) = daily_snapshot(&app) {
                eprintln!("foto diaria: {e}");
            }
            if let Err(e) = check_alerts(&app, &mut levels) {
                eprintln!("avisos: {e}");
            }
            std::thread::sleep(CHECK_EVERY);
        }
    });
}

/// Guarda una foto de la unidad del sistema si la última tiene casi un día.
fn daily_snapshot(app: &AppHandle) -> Result<(), String> {
    let root = system_root();
    let mut history = History::open_default().map_err(|e| e.to_string())?;
    let now = now_unix();
    let last = history.latest_before(&root, now + 1, None).map_err(|e| e.to_string())?;
    if last.is_some_and(|s| now - s.taken_at < SNAPSHOT_EVERY) {
        return Ok(());
    }
    // Escaneo nuevo: lo guardado en memoria puede ser de hace horas.
    let tree = app.state::<AppState>().tree(app, &root, true)?;
    let volume = system::volume_of(&root);
    history.save(&tree, volume.as_ref(), DEFAULT_MIN_FOLDER, now_unix()).map_err(|e| e.to_string())?;
    Ok(())
}

fn check_alerts(app: &AppHandle, levels: &mut HashMap<String, Level>) -> Result<(), String> {
    let volumes: Vec<_> = system::volumes()
        .into_iter()
        .filter(|v| matches!(v.kind, DriveKind::Fixed | DriveKind::Removable) && v.total > 0)
        .collect();
    let history = History::open_default().ok();
    let status =
        volume_status(&volumes, history.as_ref(), &AlertRules::default(), now_unix()).map_err(|e| e.to_string())?;
    for st in status {
        let drive = st.volume.root.display().to_string().trim_end_matches('\\').to_string();
        let before = levels.insert(drive.clone(), st.level).unwrap_or(Level::Ok);
        if st.level <= before {
            continue;
        }
        let title = match st.level {
            Level::Critical => format!("{drive} está casi llena"),
            _ => format!("{drive} se está quedando sin espacio"),
        };
        let body = if st.reasons.is_empty() {
            "Abre ItsukiManager para ver qué ocupa más.".into()
        } else {
            st.reasons.join(" · ")
        };
        let _ = app.notification().builder().title(title).body(body).show();
    }
    Ok(())
}
