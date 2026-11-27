//! Capa de presentación de escritorio de itsukiManager.
//!
//! Adapta los datos del núcleo (`itsuki-core`) a lo que muestra la interfaz y
//! ejecuta sus acciones (escanear, buscar duplicados, limpiar, enviar a la
//! Papelera). No contiene lógica de escaneo ni de limpieza propia.

mod cleanup;
mod duplicates;
mod explore;
mod home;
mod monitor;
mod settings;
mod state;
mod updater;

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, LogicalSize, Manager, WindowEvent};
use tauri_plugin_notification::NotificationExt;

/// Muestra la ventana principal (desde la bandeja o al abrir la app otra vez).
fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Icono de la bandeja: clic para abrir la ventana; menú con Abrir y Salir.
fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "abrir", "Abrir ItsukiManager", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "salir", "Salir", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("ItsukiManager")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "abrir" => show_main(app),
            "salir" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

/// La X oculta la ventana: la app sigue en la bandeja vigilando el disco. La
/// primera vez se avisa de dónde ha quedado.
fn hide_to_tray(window: &tauri::Window) {
    static TOLD: AtomicBool = AtomicBool::new(false);
    let _ = window.hide();
    if !TOLD.swap(true, Ordering::Relaxed) {
        let _ = window
            .app_handle()
            .notification()
            .builder()
            .title("ItsukiManager sigue en la bandeja")
            .body("Seguirá vigilando el espacio de tus unidades. Para cerrarla del todo, usa «Salir» en su icono.")
            .show();
    }
}

/// La ventana tiene tamaño fijo (1087 × 612, el diseño reducido un 35 %, en `tauri.conf.json`). Solo se reduce, sin perder la
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
    let state = state::AppState::default();
    *state::lock(&state.settings) = settings::Settings::load();
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main(app)))
        .plugin(tauri_plugin_notification::init())
        .manage(state)
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                hide_to_tray(window);
            }
        })
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
            settings::get_settings,
            settings::set_settings,
            settings::about_info,
            settings::open_repo,
            state::cancel_job,
            updater::update_status,
            updater::install_update,
            updater::open_release_page,
        ])
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                fit_small_screen(&window);
                let _ = window.show();
            }
            build_tray(app)?;
            monitor::start(app.handle().clone());
            updater::start(app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("no se pudo iniciar ItsukiManager");
}
