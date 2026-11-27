//! Aplicaciones abiertas: sus cachés no se tocan mientras se usan.
//!
//! Borrar la caché de un navegador o de una app de Electron mientras está
//! abierta puede dejarla en mal estado. Para cada caché de AppData se busca la
//! aplicación dueña (la carpeta tras `AppData\Local`, `AppData\Roaming` o
//! `Packages`) y se compara con las carpetas y nombres de los programas en
//! ejecución.

use std::collections::HashSet;
use std::path::{Component, Path};

/// Carpetas de fabricante: la aplicación es la carpeta siguiente.
const VENDORS: [&str; 7] = ["microsoft", "google", "nvidia", "nvidia corporation", "amd", "intel", "packages"];

/// Partes de ruta genéricas que no identifican a ninguna aplicación.
const GENERIC: [&str; 26] = [
    "windows",
    "system32",
    "syswow64",
    "program files",
    "program files (x86)",
    "programdata",
    "users",
    "appdata",
    "local",
    "roaming",
    "locallow",
    "programs",
    "windowsapps",
    "app",
    "application",
    "bin",
    "x64",
    "x86",
    "win64",
    "win32",
    "common files",
    "microsoft",
    "google",
    "packages",
    "temp",
    "cef",
];

/// Minúsculas y sin el sufijo de los paquetes de la Tienda
/// (`claude_pzs8sxrjxfjjc` → `claude`).
fn normalize(s: &str) -> String {
    let s = s.to_lowercase();
    match s.split_once('_') {
        Some((head, _)) if !head.is_empty() => head.to_string(),
        _ => s,
    }
}

fn components(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().to_lowercase()),
            _ => None,
        })
        .collect()
}

/// Aplicación dueña de una ruta de AppData, o `None` si no está en AppData.
pub fn app_owner(path: &Path) -> Option<String> {
    let comps = components(path);
    let i = comps.iter().position(|c| c == "appdata")?;
    let mut j = i + 2;
    loop {
        let c = comps.get(j)?;
        if VENDORS.contains(&c.as_str()) && comps.get(j + 1).is_some() {
            j += 1;
            continue;
        }
        return Some(normalize(c));
    }
}

/// Nombres que identifican a los programas en ejecución: las carpetas de su
/// ruta y el nombre del ejecutable (normalizados).
pub fn running_apps() -> HashSet<String> {
    let user = std::env::var("USERNAME").ok().map(|u| u.to_lowercase());
    let mut out = HashSet::new();
    for exe in process_paths() {
        let path = Path::new(&exe);
        for c in components(path) {
            out.insert(normalize(&c));
        }
        if let Some(stem) = path.file_stem() {
            out.insert(normalize(&stem.to_string_lossy()));
        }
    }
    out.retain(|s| !GENERIC.contains(&s.as_str()) && Some(s) != user.as_ref() && !s.ends_with(".exe"));
    out
}

#[cfg(windows)]
fn process_paths() -> Vec<String> {
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    };

    use crate::scan::util::Handle;

    // SAFETY: llamada sin requisitos; el handle se cierra con `Handle`.
    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snap == INVALID_HANDLE_VALUE {
        return Vec::new();
    }
    let snap = Handle(snap);
    // SAFETY: PROCESSENTRY32W es un struct C plano; todo a cero es válido.
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
    let mut out = Vec::new();
    // SAFETY: `entry` tiene el tamaño indicado.
    let mut more = unsafe { Process32FirstW(snap.0, &mut entry) } != 0;
    while more {
        // SAFETY: abrir un proceso solo para consultar su ruta.
        let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, entry.th32ProcessID) };
        let mut found = false;
        if !h.is_null() {
            let h = Handle(h);
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            // SAFETY: el búfer y su tamaño son válidos.
            if unsafe { QueryFullProcessImageNameW(h.0, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len) } != 0 {
                out.push(String::from_utf16_lossy(&buf[..len as usize]));
                found = true;
            }
        }
        if !found {
            // Sin permiso para abrirlo: al menos el nombre del ejecutable.
            let end = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            out.push(String::from_utf16_lossy(&entry.szExeFile[..end]));
        }
        // SAFETY: igual que Process32FirstW.
        more = unsafe { Process32NextW(snap.0, &mut entry) } != 0;
    }
    out
}

#[cfg(not(windows))]
fn process_paths() -> Vec<String> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner(p: &str) -> Option<String> {
        app_owner(Path::new(p))
    }

    #[test]
    fn duenos_de_cache() {
        let u = r"C:\Users\ana\AppData";
        assert_eq!(
            owner(&format!(r"{u}\Local\BraveSoftware\Brave-Browser\User Data\Default\Cache")).as_deref(),
            Some("bravesoftware")
        );
        assert_eq!(owner(&format!(r"{u}\Local\Microsoft\Edge\User Data\Default\Cache")).as_deref(), Some("edge"));
        assert_eq!(owner(&format!(r"{u}\Roaming\discord\Cache")).as_deref(), Some("discord"));
        assert_eq!(
            owner(&format!(r"{u}\Local\NVIDIA Corporation\NVIDIA App\CefCache\Default\Cache")).as_deref(),
            Some("nvidia app")
        );
        assert_eq!(
            owner(&format!(r"{u}\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude\Cache")).as_deref(),
            Some("claude")
        );
        assert_eq!(owner(&format!(r"{u}\Local\NVIDIA\DXCache")).as_deref(), Some("dxcache"));
        assert_eq!(owner(&format!(r"{u}\Local\Microsoft\Windows\INetCache")).as_deref(), Some("windows"));
        assert_eq!(owner(r"C:\Program Files (x86)\Steam\logs"), None);
    }

    #[test]
    fn nombres_de_procesos() {
        assert_eq!(normalize("Claude_1.2.3_x64__pzs8sxrjxfjjc"), "claude");
        assert_eq!(normalize("Riot Client"), "riot client");
        // Este mismo proceso de pruebas aparece en la lista.
        let me = std::env::current_exe().unwrap();
        let stem = normalize(&me.file_stem().unwrap().to_string_lossy());
        let running = running_apps();
        if cfg!(windows) {
            assert!(running.contains(&stem), "{stem} no está en {running:?}");
            assert!(!running.contains("windows"), "las partes genéricas no cuentan");
        }
    }
}
