//! Utilidades compartidas por los motores de escaneo.

/// Bit "name surrogate" de las etiquetas de reparse: uniones, enlaces
/// simbólicos y puntos de montaje. Esas carpetas no se recorren.
pub(crate) const REPARSE_TAG_NAME_SURROGATE: u32 = 0x2000_0000;

/// Diferencia entre la época de FILETIME (1601) y la Unix (1970), en 100 ns.
const FILETIME_UNIX_DIFF: i64 = 116_444_736_000_000_000;

/// FILETIME (100 ns desde 1601) a segundos Unix. 0 si es anterior a 1970.
pub(crate) fn filetime_to_unix(ft: i64) -> i64 {
    if ft <= FILETIME_UNIX_DIFF { 0 } else { (ft - FILETIME_UNIX_DIFF) / 10_000_000 }
}

/// Escribe un nombre UTF-16 como UTF-8 al final de `out`. Si contiene
/// surrogates sueltos (inválidos pero posibles en NTFS) se usa la
/// codificación de `OsStr`, para poder reconstruir la ruta exacta.
pub(crate) fn push_name(out: &mut Vec<u8>, name: &[u16]) {
    if name.iter().all(|&c| c < 0x80) {
        out.extend(name.iter().map(|&c| c as u8));
        return;
    }
    let start = out.len();
    for ch in char::decode_utf16(name.iter().copied()) {
        match ch {
            Ok(c) => {
                let mut tmp = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut tmp).as_bytes());
            }
            Err(_) => {
                out.truncate(start);
                push_invalid(out, name);
                return;
            }
        }
    }
}

#[cfg(windows)]
fn push_invalid(out: &mut Vec<u8>, name: &[u16]) {
    use std::os::windows::ffi::OsStringExt;
    out.extend_from_slice(std::ffi::OsString::from_wide(name).as_encoded_bytes());
}

#[cfg(not(windows))]
fn push_invalid(out: &mut Vec<u8>, name: &[u16]) {
    out.extend_from_slice(String::from_utf16_lossy(name).as_bytes());
}

#[cfg(windows)]
pub(crate) use win::*;

#[cfg(windows)]
mod win {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};

    /// Ruta en formato `\\?\` terminada en NUL (evita el límite de 260 caracteres).
    pub(crate) fn verbatim(path: &Path) -> Vec<u16> {
        let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        let starts = |prefix: &str| {
            let p: Vec<u16> = prefix.encode_utf16().collect();
            wide.starts_with(&p)
        };
        let mut out = Vec::with_capacity(wide.len() + 9);
        if starts(r"\\?\") || starts(r"\\.\") {
            out.extend_from_slice(&wide);
        } else if starts(r"\\") {
            out.extend(r"\\?\UNC\".encode_utf16());
            out.extend_from_slice(&wide[2..]);
        } else {
            out.extend(r"\\?\".encode_utf16());
            out.extend_from_slice(&wide);
        }
        out.push(0);
        out
    }

    /// Handle de Windows que se cierra solo.
    pub(crate) struct Handle(pub HANDLE);

    // SAFETY: un handle de Windows se puede usar desde cualquier hilo.
    unsafe impl Send for Handle {}
    unsafe impl Sync for Handle {}

    impl Handle {
        /// Convierte el resultado de `CreateFileW` en un `Handle` o en el error del sistema.
        pub(crate) fn from_create(h: HANDLE) -> std::io::Result<Handle> {
            if h == INVALID_HANDLE_VALUE { Err(std::io::Error::last_os_error()) } else { Ok(Handle(h)) }
        }
    }

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: el handle es válido y solo se cierra aquí.
            unsafe { CloseHandle(self.0) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn nombres_utf16_a_utf8() {
        for s in ["simple.txt", "ñandú.png", "日本語", "emoji 😀.mp4"] {
            let mut out = Vec::new();
            push_name(&mut out, &wide(s));
            assert_eq!(out, s.as_bytes());
        }
    }

    #[cfg(windows)]
    #[test]
    fn surrogate_suelto_se_conserva() {
        use std::os::windows::ffi::OsStrExt;
        let raw = [b'a' as u16, 0xD800, b'b' as u16];
        let mut out = Vec::new();
        push_name(&mut out, &raw);
        // SAFETY: los bytes salen de as_encoded_bytes.
        let os = unsafe { std::ffi::OsStr::from_encoded_bytes_unchecked(&out) };
        assert_eq!(os.encode_wide().collect::<Vec<_>>(), raw);
    }

    #[cfg(windows)]
    #[test]
    fn rutas_verbatim() {
        use std::path::Path;
        let s = |p: &str| String::from_utf16(&verbatim(Path::new(p))[..]).unwrap();
        assert_eq!(s(r"C:\Users"), "\\\\?\\C:\\Users\0");
        assert_eq!(s(r"\\srv\share\x"), "\\\\?\\UNC\\srv\\share\\x\0");
        assert_eq!(s(r"\\?\D:\"), "\\\\?\\D:\\\0");
    }

    #[test]
    fn fechas_filetime() {
        assert_eq!(filetime_to_unix(0), 0);
        assert_eq!(filetime_to_unix(FILETIME_UNIX_DIFF + 10_000_000), 1);
    }
}
