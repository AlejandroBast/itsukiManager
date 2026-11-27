//! Capa de presentación de escritorio de itsukiManager.
//!
//! Adapta los datos del núcleo (`itsuki-core`) a lo que muestra la interfaz y
//! ejecuta sus acciones (escanear, buscar duplicados, limpiar, enviar a la
//! Papelera). No contiene lógica de escaneo ni de limpieza propia.

mod cleanup;
mod duplicates;
mod explore;
mod home;
mod state;

use tauri::{LogicalSize, Manager};

/// La ventana tiene tamaño fijo (1672 × 941, el del diseño, en `tauri.conf.json`). Solo se reduce, sin perder la
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
        .manage(state::AppState::default())
        .invoke_handler(tauri::generate_handler![
            home::system_disk,
            home::list_drives,
            home::home_summary,
            explore::large_folders,
            explore::large_files,
            explore::reveal_path,
            explore::recycle_paths,
            duplicates::find_dupes,
            cleanup::junk_scan,
            cleanup::junk_clean,
            state::cancel_job,
        ])
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
