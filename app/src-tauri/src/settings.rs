//! Secciones «Configuración» y «Acerca de»: ajustes guardados en
//! `ajustes.json` (junto al historial) e información del sistema.

use std::path::PathBuf;

use itsuki_core::clean::{Action, JunkCategory};
use itsuki_core::history::History;
use itsuki_core::{Engine, JunkContext, ScanOptions, system};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::home::{DiskInfo, list_drives};
use crate::state::{AppState, lock};

const REPO: &str = "https://github.com/AlejandroBast/itsukiManager";
const DAY: i64 = 86_400;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EngineChoice {
    Auto,
    Mft,
    Native,
    Portable,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub engine: EngineChoice,
    /// Contar una sola vez los archivos con varios enlaces duros.
    pub dedupe_hardlinks: bool,
    /// Valor inicial de «Enviar a la Papelera» en Limpieza.
    pub clean_to_recycle: bool,
    /// Valor inicial de «Incluir apps abiertas» en Limpieza.
    pub clean_include_open: bool,
    /// Solo se borran los temporales con al menos esta antigüedad.
    pub temp_min_age_days: u32,
    /// Valores iniciales de Duplicados.
    pub dupes_scope: String,
    pub dupes_min: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            engine: EngineChoice::Auto,
            dedupe_hardlinks: true,
            clean_to_recycle: false,
            clean_include_open: false,
            temp_min_age_days: 1,
            dupes_scope: "personal".into(),
            dupes_min: 1024 * 1024,
        }
    }
}

impl Settings {
    pub fn scan_options(&self) -> ScanOptions {
        let engine = match self.engine {
            EngineChoice::Auto => Engine::Auto,
            EngineChoice::Mft => Engine::Mft,
            EngineChoice::Native => Engine::Native,
            EngineChoice::Portable => Engine::Portable,
        };
        ScanOptions { engine, dedupe_hardlinks: self.dedupe_hardlinks, ..ScanOptions::default() }
    }

    /// Contexto de limpieza con la antigüedad mínima de temporales elegida.
    pub fn junk_context(&self) -> JunkContext {
        let mut ctx = JunkContext::from_env();
        let age = i64::from(self.temp_min_age_days) * DAY;
        ctx.temp_min_age = age;
        for loc in &mut ctx.locations {
            if loc.category == JunkCategory::Temp {
                if let Action::Contents { min_age } = &mut loc.action {
                    *min_age = age;
                }
            }
        }
        ctx
    }

    fn path() -> PathBuf {
        History::default_dir().join("ajustes.json")
    }

    /// Ajustes guardados; si no hay o están dañados, los de por defecto.
    pub fn load() -> Settings {
        std::fs::read(Self::path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    fn save(&self) -> Result<(), String> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, json).map_err(|e| format!("no se pudieron guardar los ajustes: {e}"))
    }
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    lock(&state.settings).clone()
}

/// Guarda los ajustes. Si cambia cómo se escanea, se olvida lo escaneado para
/// que el próximo análisis lo use.
#[tauri::command]
pub fn set_settings(state: State<'_, AppState>, settings: Settings) -> Result<Settings, String> {
    settings.save()?;
    let old = std::mem::replace(&mut *lock(&state.settings), settings.clone());
    if old.engine != settings.engine || old.dedupe_hardlinks != settings.dedupe_hardlinks {
        state.forget_scans();
    } else if old.temp_min_age_days != settings.temp_min_age_days {
        *lock(&state.junk) = None;
        *lock(&state.home) = None;
    }
    Ok(settings)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveEngine {
    #[serde(flatten)]
    disk: DiskInfo,
    /// Motor que usará el escaneo automático de la unidad entera.
    engine: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AboutInfo {
    version: String,
    repo: &'static str,
    elevated: bool,
    threads: usize,
    data_dir: String,
    drives: Vec<DriveEngine>,
}

#[tauri::command]
pub fn about_info(app: AppHandle) -> AboutInfo {
    let elevated = system::is_elevated();
    let drives = list_drives()
        .into_iter()
        .map(|disk| {
            let engine = if elevated && disk.filesystem.eq_ignore_ascii_case("NTFS") { "MFT" } else { "Nativo" };
            DriveEngine { disk, engine }
        })
        .collect();
    AboutInfo {
        version: app.package_info().version.to_string(),
        repo: REPO,
        elevated,
        threads: std::thread::available_parallelism().map_or(1, |n| n.get()),
        data_dir: History::default_dir().display().to_string(),
        drives,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn antiguedad_de_temporales() {
        let s = Settings { temp_min_age_days: 7, ..Settings::default() };
        let ctx = s.junk_context();
        assert_eq!(ctx.temp_min_age, 7 * DAY);
        let temps: Vec<_> = ctx.locations.iter().filter(|l| l.category == JunkCategory::Temp).collect();
        assert!(!temps.is_empty());
        assert!(temps.iter().all(|l| matches!(l.action, Action::Contents { min_age } if min_age == 7 * DAY)));
    }

    #[test]
    fn ajustes_incompletos_usan_los_de_por_defecto() {
        let s: Settings = serde_json::from_str(r#"{"engine":"native"}"#).unwrap();
        assert!(s.engine == EngineChoice::Native && s.dedupe_hardlinks && s.dupes_scope == "personal");
    }
}

/// Abre el repositorio en el navegador predeterminado.
#[tauri::command]
pub fn open_repo() -> Result<(), String> {
    std::process::Command::new("explorer.exe").arg(REPO).spawn().map(|_| ()).map_err(|e| e.to_string())
}
