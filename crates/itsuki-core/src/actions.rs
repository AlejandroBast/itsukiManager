//! Acciones sobre archivos con protecciones: enviar a la Papelera y mostrar en
//! el Explorador. Nunca se borra nada de forma permanente.

use std::io;
use std::path::{Path, PathBuf};

/// Motivo por el que una ruta no se puede enviar a la Papelera, o `None` si
/// se permite. Protege la raíz de las unidades, Windows y las carpetas base
/// del sistema y del usuario.
pub fn protection_reason(path: &Path) -> Option<&'static str> {
    // En Windows "C:" es la carpeta actual de esa unidad: se rechaza por ambigua.
    let raw = path.as_os_str().as_encoded_bytes();
    if raw.len() == 2 && raw[1] == b':' && raw[0].is_ascii_alphabetic() {
        return Some("es la raíz de una unidad");
    }
    let Ok(abs) = std::path::absolute(path) else {
        return Some("la ruta no es válida");
    };
    let p = normalize(&abs);
    if is_drive_root(&p) {
        return Some("es la raíz de una unidad");
    }

    let env = |key: &str| std::env::var_os(key).map(|v| normalize(Path::new(&v)));
    if let Some(win) = env("SystemRoot") {
        if p == win || p.starts_with(&format!("{win}\\")) {
            return Some("es parte de Windows");
        }
    }
    let bases = [
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramW6432",
        "ProgramData",
        "PUBLIC",
        "APPDATA",
        "LOCALAPPDATA",
        "USERPROFILE",
        "OneDrive",
    ];
    if bases.iter().filter_map(|k| env(k)).any(|b| p == b) {
        return Some("es una carpeta base del sistema");
    }
    if let Some(profile) = env("USERPROFILE") {
        if let Some(users) = Path::new(&profile).parent().map(normalize) {
            if p == users {
                return Some("es la carpeta de usuarios");
            }
        }
        let personal = [
            "desktop",
            "documents",
            "downloads",
            "pictures",
            "videos",
            "music",
            "appdata",
            "appdata\\local",
            "appdata\\roaming",
            "appdata\\locallow",
        ];
        if personal.iter().any(|f| p == format!("{profile}\\{f}")) {
            return Some("es una carpeta personal completa");
        }
    }
    if let Some((drive, rest)) = p.split_once(":\\") {
        if drive.len() == 1 {
            let root_items = [
                "$recycle.bin",
                "system volume information",
                "recovery",
                "boot",
                "bootmgr",
                "efi",
                "pagefile.sys",
                "hiberfil.sys",
                "swapfile.sys",
                "dumpstack.log.tmp",
                "$winreagent",
                "$windows.~bt",
                "$windows.~ws",
                "config.msi",
                "users",
                "windows",
            ];
            let first = rest.split('\\').next().unwrap_or("");
            if root_items.contains(&rest) || (first == "system volume information") {
                return Some("es un elemento del sistema en la raíz de la unidad");
            }
        }
    }
    None
}

/// Minúsculas, separadores `\` y sin `\` final (salvo en `c:\`).
fn normalize(path: &Path) -> String {
    let mut s = path.to_string_lossy().replace('/', "\\").to_lowercase();
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        s = rest.to_string();
    }
    while s.ends_with('\\') && !is_drive_root(&s) {
        s.pop();
    }
    s
}

fn is_drive_root(p: &str) -> bool {
    let b = p.as_bytes();
    (b.len() == 2 || (b.len() == 3 && b[2] == b'\\')) && b[0].is_ascii_alphabetic() && b[1] == b':'
        || p == "\\"
        || p == "/"
}

/// Envía archivos o carpetas a la Papelera de reciclaje.
///
/// Rechaza las rutas protegidas ([`protection_reason`]). Si algún elemento no
/// cabe en la Papelera, Windows pide confirmación antes de borrarlo del todo.
#[cfg(windows)]
pub fn send_to_recycle_bin(paths: &[PathBuf]) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;
    use windows_sys::Win32::System::Com::{
        COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
    };
    use windows_sys::Win32::UI::Shell::{
        FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FOF_WANTNUKEWARNING, SHFILEOPSTRUCTW,
        SHFileOperationW,
    };

    if paths.is_empty() {
        return Ok(());
    }
    let mut from: Vec<u16> = Vec::new();
    for p in paths {
        if let Some(reason) = protection_reason(p) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("{} está protegida: {reason}", p.display()),
            ));
        }
        std::fs::symlink_metadata(p).map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", p.display())))?;
        let abs = std::path::absolute(p)?;
        from.extend(abs.as_os_str().encode_wide());
        from.push(0);
    }
    from.push(0); // la lista termina con doble NUL

    // SAFETY: inicialización COM del hilo actual; se deshace solo si tuvo éxito.
    let com = unsafe { CoInitializeEx(ptr::null(), (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32) };
    let mut op = SHFILEOPSTRUCTW {
        hwnd: ptr::null_mut(),
        wFunc: FO_DELETE,
        pFrom: from.as_ptr(),
        pTo: ptr::null(),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_WANTNUKEWARNING | FOF_NOERRORUI | FOF_SILENT) as u16,
        fAnyOperationsAborted: 0,
        hNameMappings: ptr::null_mut(),
        lpszProgressTitle: ptr::null(),
    };
    // SAFETY: `from` vive durante la llamada y termina en doble NUL.
    let result = unsafe { SHFileOperationW(&mut op) };
    if com >= 0 {
        // SAFETY: equilibra el CoInitializeEx que tuvo éxito.
        unsafe { CoUninitialize() };
    }
    if result != 0 {
        return Err(io::Error::other(format!("la Papelera rechazó la operación (código 0x{result:X})")));
    }
    if op.fAnyOperationsAborted != 0 {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "operación cancelada"));
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn send_to_recycle_bin(_paths: &[PathBuf]) -> io::Result<()> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "la Papelera solo está disponible en Windows"))
}

/// Abre el Explorador con el elemento seleccionado.
#[cfg(windows)]
pub fn reveal_in_explorer(path: &Path) -> io::Result<()> {
    use std::os::windows::process::CommandExt;
    let abs = std::path::absolute(path)?;
    std::fs::symlink_metadata(&abs)?;
    std::process::Command::new("explorer.exe").raw_arg(format!("/select,\"{}\"", abs.display())).spawn()?;
    Ok(())
}

#[cfg(not(windows))]
pub fn reveal_in_explorer(_path: &Path) -> io::Result<()> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "solo disponible en Windows"))
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn rutas_protegidas() {
        let windir = std::env::var("SystemRoot").unwrap();
        let profile = std::env::var("USERPROFILE").unwrap();
        assert!(protection_reason(Path::new("C:\\")).is_some());
        assert!(protection_reason(Path::new("c:")).is_some());
        assert!(protection_reason(Path::new(&windir)).is_some());
        assert!(protection_reason(&Path::new(&windir).join("System32").join("drivers")).is_some());
        assert!(protection_reason(Path::new(&profile)).is_some());
        assert!(protection_reason(&Path::new(&profile).join("Documents")).is_some());
        assert!(protection_reason(&Path::new(&profile).join("AppData").join("Local")).is_some());
        assert!(protection_reason(Path::new("C:\\pagefile.sys")).is_some());
        assert!(protection_reason(Path::new("C:\\System Volume Information\\x")).is_some());
        assert!(protection_reason(Path::new("C:\\Program Files")).is_some());
    }

    #[test]
    fn rutas_permitidas() {
        let profile = std::env::var("USERPROFILE").unwrap();
        assert_eq!(protection_reason(&Path::new(&profile).join("Downloads").join("juego.iso")), None);
        assert_eq!(protection_reason(&Path::new(&profile).join("AppData").join("Local").join("Temp")), None);
        assert_eq!(protection_reason(Path::new("C:\\Program Files\\Epic Games\\Fortnite")), None);
        assert_eq!(protection_reason(Path::new("D:\\Peliculas")), None);
    }
}
