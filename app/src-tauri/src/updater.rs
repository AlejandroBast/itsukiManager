//! Actualizaciones desde las releases de GitHub. Se comprueban al abrir y una vez al día; con un clic se
//! descarga la versión nueva, se verifica su SHA-256 y se aplica: la instalada ejecuta el instalador en
//! silencio y la portable reemplaza sus propios archivos. En los dos casos la app se reinicia sola.

use std::fs;
use std::io::{Read, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use itsuki_core::format::now_unix;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};

use crate::state::{AppState, lock, run_blocking};

const LATEST: &str = "https://api.github.com/repos/AlejandroBast/itsukiManager/releases/latest";
const RELEASES: &str = "https://github.com/AlejandroBast/itsukiManager/releases/";
const SETUP_SUFFIX: &str = "_x64-setup.exe";
const PORTABLE_SUFFIX: &str = "_x64_portable.zip";
/// Nombre del ejecutable de la app dentro del zip portable.
const PORTABLE_EXE: &str = "ItsukiManager.exe";
const FIRST_CHECK: Duration = Duration::from_secs(15);
const CHECK_EVERY: Duration = Duration::from_secs(24 * 3600);
const MAX_DOWNLOAD: u64 = 300 * 1024 * 1024;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    current: String,
    latest: String,
    available: bool,
    /// Página de la release (novedades).
    url: String,
    /// Tamaño de lo que se descargaría (instalador o zip portable).
    size: u64,
    portable: bool,
    checked_at: i64,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    assets: Vec<Asset>,
}

#[derive(Clone, Deserialize)]
struct Asset {
    name: String,
    size: u64,
    browser_download_url: String,
    /// `sha256:<hex>`, calculado por GitHub al subir el archivo.
    digest: Option<String>,
}

#[derive(Clone, Serialize)]
struct Progress {
    done: u64,
    total: u64,
}

fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let mut parts = s.trim().trim_start_matches('v').split('.').map(|p| {
        let digits: String = p.chars().take_while(char::is_ascii_digit).collect();
        digits.parse::<u64>().ok()
    });
    Some((parts.next()??, parts.next()??, parts.next().flatten().unwrap_or(0)))
}

/// La instalación con NSIS deja `uninstall.exe` junto al ejecutable; la portable no.
fn is_portable() -> bool {
    std::env::current_exe().ok().and_then(|e| e.parent().map(|d| !d.join("uninstall.exe").exists())).unwrap_or(true)
}

fn user_agent(app: &AppHandle) -> String {
    format!("ItsukiManager/{}", app.package_info().version)
}

fn fetch_release(app: &AppHandle) -> Result<Release, String> {
    let mut resp = ureq::get(LATEST)
        .header("User-Agent", &user_agent(app))
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("no se pudo consultar GitHub: {e}"))?;
    resp.body_mut().read_json::<Release>().map_err(|e| format!("respuesta de GitHub no válida: {e}"))
}

fn asset_for(release: &Release, portable: bool) -> Option<&Asset> {
    let suffix = if portable { PORTABLE_SUFFIX } else { SETUP_SUFFIX };
    release.assets.iter().find(|a| a.name.ends_with(suffix))
}

fn check(app: &AppHandle) -> Result<UpdateInfo, String> {
    let release = fetch_release(app)?;
    let current = app.package_info().version.to_string();
    let portable = is_portable();
    let newer = match (parse_version(&release.tag_name), parse_version(&current)) {
        (Some(latest), Some(now)) => latest > now,
        _ => false,
    };
    let asset = asset_for(&release, portable);
    let info = UpdateInfo {
        current,
        latest: release.tag_name.trim_start_matches('v').to_string(),
        available: newer && asset.is_some(),
        url: release.html_url.clone(),
        size: asset.map_or(0, |a| a.size),
        portable,
        checked_at: now_unix(),
    };
    *lock(&app.state::<AppState>().update) = Some(info.clone());
    Ok(info)
}

/// Comprueba al abrir y luego una vez al día. Sin conexión no pasa nada: se reintenta al día siguiente.
pub fn start(app: AppHandle) {
    cleanup_old_files();
    std::thread::spawn(move || {
        std::thread::sleep(FIRST_CHECK);
        loop {
            if let Err(e) = check(&app) {
                eprintln!("actualizaciones: {e}");
            }
            std::thread::sleep(CHECK_EVERY);
        }
    });
}

/// Estado de las actualizaciones. Con `force` (o si aún no se comprobó) se consulta GitHub.
#[tauri::command]
pub async fn update_status(app: AppHandle, force: bool) -> Result<UpdateInfo, String> {
    run_blocking(move || {
        if !force {
            if let Some(info) = lock(&app.state::<AppState>().update).clone() {
                return Ok(info);
            }
        }
        check(&app)
    })
    .await
}

/// Abre la página de la última release en el navegador.
#[tauri::command]
pub fn open_release_page(app: AppHandle) -> Result<(), String> {
    let url = lock(&app.state::<AppState>().update).as_ref().map(|i| i.url.clone());
    let url = url.filter(|u| u.starts_with(RELEASES)).unwrap_or_else(|| format!("{RELEASES}latest"));
    Command::new("explorer.exe").arg(url).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Descarga `asset` en `dir`, avisando del progreso, y comprueba su SHA-256.
fn download(app: &AppHandle, asset: &Asset, dir: &Path) -> Result<PathBuf, String> {
    let resp = ureq::get(&asset.browser_download_url)
        .header("User-Agent", &user_agent(app))
        .call()
        .map_err(|e| format!("no se pudo descargar la actualización: {e}"))?;
    let mut reader = resp.into_body().into_with_config().limit(MAX_DOWNLOAD).reader();
    let path = dir.join(&asset.name);
    let mut file = fs::File::create(&path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut done = 0u64;
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("se cortó la descarga: {e}"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        hasher.update(&buf[..n]);
        done += n as u64;
        let _ = app.emit("update-progress", Progress { done, total: asset.size });
    }
    file.flush().map_err(|e| e.to_string())?;
    if done != asset.size {
        return Err("la descarga quedó incompleta".into());
    }
    let hex: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    match asset.digest.as_deref().and_then(|d| d.strip_prefix("sha256:")) {
        Some(expected) if !expected.eq_ignore_ascii_case(&hex) => {
            Err("la descarga no coincide con la publicada (SHA-256 distinto); no se instaló nada".into())
        }
        _ => Ok(path),
    }
}

/// Sufijo de los archivos sustituidos; solo se borran estos (la portable puede estar en el Escritorio).
const OLD_SUFFIX: &str = ".itsuki-old";

/// Sustituye `target` por `new` aunque `target` esté en uso: el actual se renombra y se borra al abrir.
fn replace_file(new: &Path, target: &Path) -> Result<(), String> {
    if target.exists() {
        let mut old = target.as_os_str().to_owned();
        old.push(OLD_SUFFIX);
        let old = PathBuf::from(old);
        let _ = fs::remove_file(&old);
        fs::rename(target, &old).map_err(|e| format!("no se pudo reemplazar {}: {e}", target.display()))?;
    }
    fs::copy(new, target).map(|_| ()).map_err(|e| format!("no se pudo copiar {}: {e}", target.display()))
}

/// Extrae el zip portable en una carpeta temporal y coloca sus archivos junto al ejecutable actual.
fn apply_portable(zip_path: &Path, work: &Path, exe: &Path) -> Result<(), String> {
    let dir = exe.parent().ok_or("carpeta de la app desconocida")?;
    let file = fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("zip no válido: {e}"))?;
    let out = work.join("extraido");
    let _ = fs::remove_dir_all(&out);
    fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let mut files = Vec::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let Some(name) = entry.enclosed_name().and_then(|p| p.file_name().map(|n| n.to_owned())) else { continue };
        if entry.is_dir() {
            continue;
        }
        let path = out.join(&name);
        let mut f = fs::File::create(&path).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut f).map_err(|e| e.to_string())?;
        files.push((name, path));
    }
    if !files.iter().any(|(n, _)| n == PORTABLE_EXE) {
        return Err(format!("el zip no trae {PORTABLE_EXE}"));
    }
    for (name, path) in &files {
        let target = if name == PORTABLE_EXE { exe.to_path_buf() } else { dir.join(name) };
        replace_file(path, &target)?;
    }
    Ok(())
}

fn ps_quote(p: &Path) -> String {
    format!("'{}'", p.display().to_string().replace('\'', "''"))
}

/// Lanza, oculto y desligado de la app, lo que debe ocurrir cuando ya se haya cerrado.
fn after_exit(script: &str) -> Result<(), String> {
    Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", script])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("no se pudo preparar el reinicio: {e}"))
}

/// Descarga y aplica la última versión; después la app se cierra y se vuelve a abrir actualizada.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    run_blocking(move || {
        let release = fetch_release(&app)?;
        let portable = is_portable();
        let asset = asset_for(&release, portable).ok_or("la versión publicada no trae el archivo necesario")?.clone();
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let work = std::env::temp_dir().join("itsukiManager-actualizacion");
        fs::create_dir_all(&work).map_err(|e| e.to_string())?;
        let downloaded = download(&app, &asset, &work)?;
        let pid = std::process::id();
        let script = if portable {
            apply_portable(&downloaded, &work, &exe)?;
            format!("Wait-Process -Id {pid} -ErrorAction SilentlyContinue; Start-Process -FilePath {}", ps_quote(&exe))
        } else {
            format!(
                "Wait-Process -Id {pid} -ErrorAction SilentlyContinue; Start-Process -FilePath {} -ArgumentList '/S' -Wait; Start-Process -FilePath {}",
                ps_quote(&downloaded),
                ps_quote(&exe)
            )
        };
        after_exit(&script)?;
        let handle = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(600));
            handle.exit(0);
        });
        Ok(())
    })
    .await
}

/// Borra los archivos sustituidos que deja la actualización de la portable.
fn cleanup_old_files() {
    let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) else { return };
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            if e.file_name().to_string_lossy().ends_with(OLD_SUFFIX) {
                let _ = fs::remove_file(e.path());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versiones() {
        assert_eq!(parse_version("v1.1.0"), Some((1, 1, 0)));
        assert_eq!(parse_version("1.2"), Some((1, 2, 0)));
        assert_eq!(parse_version("v2.0.1-beta"), Some((2, 0, 1)));
        assert!(parse_version("v1.10.0") > parse_version("v1.9.9"));
        assert_eq!(parse_version("latest"), None);
    }
}
