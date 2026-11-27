//! Estado compartido entre secciones: los árboles ya escaneados (se reutilizan
//! para no volver a recorrer el disco) y el trabajo largo en curso, que se
//! puede cancelar.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use itsuki_core::{DupProgress, ScanError, ScanProgress, Tree, scan};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::cleanup::JunkState;
use crate::home::HomeSummary;
use crate::settings::Settings;

/// Mensaje de error cuando el usuario cancela (la interfaz no lo muestra como fallo).
pub const CANCELLED: &str = "cancelado";

/// Trabajo largo que se puede cancelar.
enum Job {
    Scan(Arc<ScanProgress>),
    Dupes(Arc<DupProgress>),
}

#[derive(Default)]
pub struct AppState {
    trees: Mutex<HashMap<String, Arc<Tree>>>,
    /// Un escaneo a la vez: quien llega después espera y reutiliza el resultado.
    scanning: Mutex<()>,
    job: Mutex<Option<Job>>,
    pub home: Mutex<Option<HomeSummary>>,
    pub junk: Mutex<Option<JunkState>>,
    pub settings: Mutex<Settings>,
}

/// Bloquea sin fallar si otro hilo se interrumpió con el candado tomado.
pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Clave de una ruta: minúsculas, `\` y sin barra final (`c:`, `c:\users`).
pub fn key_of(path: &Path) -> String {
    let mut s = path.to_string_lossy().replace('/', "\\").to_lowercase();
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        s = rest.to_string();
    }
    while s.ends_with('\\') {
        s.pop();
    }
    s
}

/// `true` si `path` es `base` o está dentro.
pub fn within(path: &str, base: &str) -> bool {
    path == base || path.strip_prefix(base).is_some_and(|rest| rest.starts_with('\\'))
}

/// Raíz de la unidad del sistema (normalmente `C:\`).
pub fn system_root() -> PathBuf {
    let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
    PathBuf::from(format!("{drive}\\"))
}

#[derive(Clone, Serialize)]
pub struct ProgressEvent {
    pub files: u64,
    pub bytes: u64,
}

/// Ejecuta `f` fuera del hilo de la interfaz (los escaneos y borrados bloquean).
pub async fn run_blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

/// Ejecuta `work` enviando `event` con el progreso cada 150 ms.
pub fn with_progress<T>(
    app: &AppHandle,
    event: &'static str,
    read: impl Fn() -> ProgressEvent + Send + 'static,
    work: impl FnOnce() -> T,
) -> T {
    let done = Arc::new(AtomicBool::new(false));
    let ticker = {
        let (app, done) = (app.clone(), done.clone());
        std::thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                let _ = app.emit(event, read());
                std::thread::sleep(Duration::from_millis(150));
            }
        })
    };
    let out = work();
    done.store(true, Ordering::Relaxed);
    let _ = ticker.join();
    out
}

impl AppState {
    /// Árbol de `root`: el ya escaneado o uno nuevo (con `refresh`, siempre nuevo).
    pub fn tree(&self, app: &AppHandle, root: &Path, refresh: bool) -> Result<Arc<Tree>, String> {
        let key = key_of(root);
        if !refresh {
            if let Some(t) = lock(&self.trees).get(&key) {
                return Ok(t.clone());
            }
        }
        let _one = lock(&self.scanning);
        if !refresh {
            // Puede haberlo escaneado quien tenía el turno.
            if let Some(t) = lock(&self.trees).get(&key) {
                return Ok(t.clone());
            }
        }
        let options = lock(&self.settings).scan_options();
        let progress = Arc::new(ScanProgress::new());
        *lock(&self.job) = Some(Job::Scan(progress.clone()));
        let reader = progress.clone();
        let result = with_progress(
            app,
            "scan-progress",
            move || {
                let p = reader.snapshot();
                ProgressEvent { files: p.files, bytes: p.bytes }
            },
            || scan(root, &options, &progress),
        );
        *lock(&self.job) = None;
        let tree = Arc::new(result.map_err(|e| match e {
            ScanError::Cancelled => CANCELLED.to_string(),
            e => e.to_string(),
        })?);
        lock(&self.trees).insert(key, tree.clone());
        Ok(tree)
    }

    /// Árbol ya escaneado que contiene `path`, si lo hay.
    pub fn cached_containing(&self, path: &Path) -> Option<Arc<Tree>> {
        let p = key_of(path);
        lock(&self.trees).iter().find(|(k, _)| within(&p, k)).map(|(_, t)| t.clone())
    }

    /// Registra la búsqueda de duplicados en curso (para poder cancelarla).
    pub fn set_dupes_job(&self, progress: Option<Arc<DupProgress>>) {
        *lock(&self.job) = progress.map(Job::Dupes);
    }

    /// Olvida lo escaneado que contiene `path` o está dentro (tras borrar o
    /// limpiar): la próxima consulta vuelve a escanear.
    pub fn invalidate(&self, path: &Path) {
        let p = key_of(path);
        lock(&self.trees).retain(|k, _| !within(&p, k) && !within(k, &p));
        *lock(&self.home) = None;
        *lock(&self.junk) = None;
    }

    /// Olvida todo lo escaneado (tras cambiar cómo se escanea).
    pub fn forget_scans(&self) {
        lock(&self.trees).clear();
        *lock(&self.home) = None;
        *lock(&self.junk) = None;
    }
}

/// Cancela el escaneo o la búsqueda de duplicados en curso.
#[tauri::command]
pub fn cancel_job(state: State<'_, AppState>) {
    match lock(&state.job).as_ref() {
        Some(Job::Scan(p)) => p.cancel(),
        Some(Job::Dupes(p)) => p.cancel(),
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claves_y_contencion() {
        assert_eq!(key_of(Path::new("C:\\")), "c:");
        assert_eq!(key_of(Path::new(r"\\?\C:\Users\Ana\")), "c:\\users\\ana");
        assert!(within("c:\\users\\ana", "c:"));
        assert!(within("c:", "c:"));
        assert!(!within("c:\\users2", "c:\\users"));
        assert!(!within("d:\\juegos", "c:"));
    }
}
