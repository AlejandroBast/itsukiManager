//! Información del sistema: permisos y unidades.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// Tipo de unidad.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum DriveKind {
    Fixed,
    Removable,
    Network,
    Optical,
    Ram,
    Unknown,
}

impl DriveKind {
    pub fn label(self) -> &'static str {
        match self {
            DriveKind::Fixed => "local",
            DriveKind::Removable => "extraíble",
            DriveKind::Network => "red",
            DriveKind::Optical => "óptica",
            DriveKind::Ram => "RAM",
            DriveKind::Unknown => "desconocida",
        }
    }
}

/// Datos de una unidad montada.
#[derive(Clone, Debug, Serialize)]
pub struct Volume {
    /// Raíz, por ejemplo `C:\`.
    pub root: PathBuf,
    pub label: String,
    pub filesystem: String,
    pub total: u64,
    pub free: u64,
    pub kind: DriveKind,
    /// `Some(true)` en discos mecánicos (HDD), `Some(false)` en SSD.
    pub seek_penalty: Option<bool>,
}

impl Volume {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.free)
    }
}

#[cfg(windows)]
pub use win::*;

#[cfg(not(windows))]
pub use other::*;

#[cfg(windows)]
mod win {
    use super::*;
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::ptr;

    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_NOT_ALL_ASSIGNED, GetLastError, HANDLE, INVALID_HANDLE_VALUE, LUID,
    };
    use windows_sys::Win32::Security::{
        AdjustTokenPrivileges, GetTokenInformation, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED,
        TOKEN_ADJUST_PRIVILEGES, TOKEN_ELEVATION, TOKEN_PRIVILEGES, TOKEN_QUERY, TokenElevation,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDriveStringsW,
        GetVolumeInformationW, GetVolumePathNameW, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::IO::DeviceIoControl;
    use windows_sys::Win32::System::Ioctl::{
        DEVICE_SEEK_PENALTY_DESCRIPTOR, IOCTL_STORAGE_QUERY_PROPERTY, PropertyStandardQuery, STORAGE_PROPERTY_QUERY,
        StorageDeviceSeekPenaltyProperty,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: handle válido, se cierra una sola vez.
            unsafe { CloseHandle(self.0) };
        }
    }

    fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
        s.encode_wide().chain(Some(0)).collect()
    }

    fn token(access: u32) -> Option<Handle> {
        let mut h: HANDLE = ptr::null_mut();
        // SAFETY: GetCurrentProcess devuelve un pseudo-handle siempre válido.
        let ok = unsafe { OpenProcessToken(GetCurrentProcess(), access, &mut h) };
        (ok != 0).then_some(Handle(h))
    }

    /// `true` si el proceso se ejecuta como administrador.
    pub fn is_elevated() -> bool {
        let Some(t) = token(TOKEN_QUERY) else { return false };
        let mut elevation = TOKEN_ELEVATION::default();
        let mut len = 0u32;
        // SAFETY: el búfer es un TOKEN_ELEVATION del tamaño indicado.
        let ok = unsafe {
            GetTokenInformation(
                t.0,
                TokenElevation,
                (&mut elevation as *mut TOKEN_ELEVATION).cast(),
                size_of::<TOKEN_ELEVATION>() as u32,
                &mut len,
            )
        };
        ok != 0 && elevation.TokenIsElevated != 0
    }

    /// Activa `SeBackupPrivilege`, que permite listar carpetas protegidas.
    /// Solo funciona como administrador.
    pub fn enable_backup_privilege() -> bool {
        let Some(t) = token(TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY) else { return false };
        let name = wide("SeBackupPrivilege".as_ref());
        let mut luid = LUID { LowPart: 0, HighPart: 0 };
        // SAFETY: `name` termina en NUL.
        if unsafe { LookupPrivilegeValueW(ptr::null(), name.as_ptr(), &mut luid) } == 0 {
            return false;
        }
        let tp = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES { Luid: luid, Attributes: SE_PRIVILEGE_ENABLED }],
        };
        // SAFETY: `tp` es válido durante la llamada.
        let ok = unsafe { AdjustTokenPrivileges(t.0, 0, &tp, 0, ptr::null_mut(), ptr::null_mut()) };
        // SAFETY: sin requisitos.
        ok != 0 && unsafe { GetLastError() } != ERROR_NOT_ALL_ASSIGNED
    }

    /// Unidades montadas con letra.
    pub fn volumes() -> Vec<Volume> {
        let mut buf = vec![0u16; 512];
        // SAFETY: el tamaño pasado coincide con el búfer.
        let n = unsafe { GetLogicalDriveStringsW(buf.len() as u32, buf.as_mut_ptr()) } as usize;
        buf.truncate(n.min(buf.len()));
        buf.split(|&c| c == 0)
            .filter(|s| !s.is_empty())
            .filter_map(|s| volume_info(Path::new(&OsString::from_wide(s))))
            .collect()
    }

    /// Unidad que contiene `path`.
    pub fn volume_of(path: &Path) -> Option<Volume> {
        let w = wide(path.as_os_str());
        let mut buf = vec![0u16; 1024];
        // SAFETY: `w` termina en NUL; el tamaño coincide con el búfer.
        if unsafe { GetVolumePathNameW(w.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } == 0 {
            return None;
        }
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        volume_info(Path::new(&OsString::from_wide(&buf[..end])))
    }

    fn volume_info(root: &Path) -> Option<Volume> {
        let w = wide(root.as_os_str());
        // SAFETY: `w` termina en NUL.
        let kind = match unsafe { GetDriveTypeW(w.as_ptr()) } {
            2 => DriveKind::Removable,
            3 => DriveKind::Fixed,
            4 => DriveKind::Network,
            5 => DriveKind::Optical,
            6 => DriveKind::Ram,
            1 => return None, // ruta raíz inválida
            _ => DriveKind::Unknown,
        };
        let (mut free_caller, mut total, mut free) = (0u64, 0u64, 0u64);
        // SAFETY: punteros a variables locales.
        if unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut free_caller, &mut total, &mut free) } == 0 {
            return None;
        }
        let mut label = [0u16; 261];
        let mut fs = [0u16; 261];
        // SAFETY: los tamaños coinciden con los búferes.
        let ok = unsafe {
            GetVolumeInformationW(
                w.as_ptr(),
                label.as_mut_ptr(),
                label.len() as u32,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                fs.as_mut_ptr(),
                fs.len() as u32,
            )
        };
        let text = |b: &[u16]| {
            let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
            String::from_utf16_lossy(&b[..end])
        };
        let (label, filesystem) = if ok != 0 { (text(&label), text(&fs)) } else { (String::new(), String::new()) };
        let seek_penalty = if kind == DriveKind::Fixed { seek_penalty(root) } else { None };
        Some(Volume { root: root.to_path_buf(), label, filesystem, total, free, kind, seek_penalty })
    }

    /// Consulta si el disco tiene penalización por búsqueda (HDD).
    fn seek_penalty(root: &Path) -> Option<bool> {
        let s = root.to_string_lossy();
        let letter = s.chars().next().filter(|c| c.is_ascii_alphabetic())?;
        let dev = wide(format!(r"\\.\{letter}:").as_ref());
        // SAFETY: acceso 0 (solo consultas); `dev` termina en NUL.
        let h = unsafe {
            CreateFileW(
                dev.as_ptr(),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                ptr::null(),
                OPEN_EXISTING,
                0,
                ptr::null_mut(),
            )
        };
        if h == INVALID_HANDLE_VALUE {
            return None;
        }
        let h = Handle(h);
        let query = STORAGE_PROPERTY_QUERY {
            PropertyId: StorageDeviceSeekPenaltyProperty,
            QueryType: PropertyStandardQuery,
            AdditionalParameters: [0],
        };
        let mut desc = DEVICE_SEEK_PENALTY_DESCRIPTOR::default();
        let mut returned = 0u32;
        // SAFETY: búferes de entrada y salida válidos con sus tamaños.
        let ok = unsafe {
            DeviceIoControl(
                h.0,
                IOCTL_STORAGE_QUERY_PROPERTY,
                (&query as *const STORAGE_PROPERTY_QUERY).cast(),
                size_of::<STORAGE_PROPERTY_QUERY>() as u32,
                (&mut desc as *mut DEVICE_SEEK_PENALTY_DESCRIPTOR).cast(),
                size_of::<DEVICE_SEEK_PENALTY_DESCRIPTOR>() as u32,
                &mut returned,
                ptr::null_mut(),
            )
        };
        // Version (4) + Size (4) + IncursSeekPenalty (1).
        (ok != 0 && returned >= 9).then_some(desc.IncursSeekPenalty)
    }
}

#[cfg(not(windows))]
mod other {
    use super::*;

    pub fn is_elevated() -> bool {
        false
    }

    pub fn enable_backup_privilege() -> bool {
        false
    }

    pub fn volumes() -> Vec<Volume> {
        Vec::new()
    }

    pub fn volume_of(_path: &Path) -> Option<Volume> {
        None
    }
}
