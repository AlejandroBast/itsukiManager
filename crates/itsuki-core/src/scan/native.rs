//! Motor nativo de Windows.
//!
//! - Cada subcarpeta se abre *relativa al handle del padre* con `NtCreateFile`,
//!   sin rutas completas: no hay límite de 260 caracteres y el kernel no tiene
//!   que recorrer la ruta desde la raíz en cada apertura.
//! - Las carpetas se enumeran en bloques de 64 KiB con
//!   `GetFileInformationByHandleEx`, que devuelve nombre, tamaños, atributos,
//!   etiqueta de reparse e identificador de archivo en una sola llamada.
//! - Con permisos de administrador se usa la intención de copia de seguridad,
//!   que permite leer carpetas protegidas.

use std::cell::RefCell;
use std::io;
use std::path::Path;
use std::ptr::{self, addr_of};
use std::sync::atomic::{AtomicI32, Ordering};

use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
use windows_sys::Wdk::Storage::FileSystem::{
    FILE_DIRECTORY_FILE, FILE_OPEN, FILE_OPEN_FOR_BACKUP_INTENT, FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT,
    NtCreateFile,
};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_INVALID_LEVEL, ERROR_INVALID_PARAMETER, ERROR_NO_MORE_FILES, ERROR_NOT_SUPPORTED, GetLastError,
    HANDLE, INVALID_HANDLE_VALUE, OBJ_CASE_INSENSITIVE, RtlNtStatusToDosError, UNICODE_STRING,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_COMPRESSED, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_OFFLINE,
    FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS, FILE_ATTRIBUTE_RECALL_ON_OPEN, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_ATTRIBUTE_SPARSE_FILE, FILE_ATTRIBUTE_SYSTEM, FILE_FLAG_BACKUP_SEMANTICS, FILE_FULL_DIR_INFO,
    FILE_ID_EXTD_DIR_INFO, FILE_LIST_DIRECTORY, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    FileFullDirectoryInfo, FileFullDirectoryRestartInfo, FileIdExtdDirectoryInfo, GetFileInformationByHandleEx,
    OPEN_EXISTING, SYNCHRONIZE,
};
use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;

use super::util::{REPARSE_TAG_NAME_SURROGATE, filetime_to_unix, push_name, verbatim};
use super::walk::{Backend, Batch};
use crate::tree::flags;

/// Tamaño del búfer de enumeración por hilo.
const BUF_BYTES: usize = 64 * 1024;

thread_local! {
    static BUF: RefCell<Vec<u64>> = RefCell::new(vec![0u64; BUF_BYTES / 8]);
}

/// Handle de carpeta que se cierra solo.
pub(crate) struct DirHandle(HANDLE);

// SAFETY: un handle de Windows se puede usar desde cualquier hilo.
unsafe impl Send for DirHandle {}
unsafe impl Sync for DirHandle {}

impl Drop for DirHandle {
    fn drop(&mut self) {
        // SAFETY: el handle es válido y solo se cierra aquí.
        unsafe { CloseHandle(self.0) };
    }
}

pub(crate) struct NativeBackend {
    /// Clase de enumeración en uso (se degrada si el sistema de archivos no
    /// ofrece identificadores, por ejemplo en FAT32/exFAT).
    class: AtomicI32,
}

impl NativeBackend {
    pub fn new() -> Self {
        NativeBackend { class: AtomicI32::new(FileIdExtdDirectoryInfo) }
    }
}

impl Backend for NativeBackend {
    type Dir = DirHandle;
    type Key = Box<[u16]>;

    fn open_root(&self, path: &Path) -> io::Result<DirHandle> {
        let wide = verbatim(path);
        // SAFETY: `wide` termina en NUL y vive durante la llamada.
        let h = unsafe {
            CreateFileW(
                wide.as_ptr(),
                FILE_LIST_DIRECTORY | SYNCHRONIZE,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                ptr::null_mut(),
            )
        };
        if h == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        Ok(DirHandle(h))
    }

    fn open_child(&self, parent: &DirHandle, name: &Box<[u16]>) -> io::Result<DirHandle> {
        let bytes = (name.len() * 2) as u16;
        let object_name = UNICODE_STRING { Length: bytes, MaximumLength: bytes, Buffer: name.as_ptr() as *mut u16 };
        let attributes = OBJECT_ATTRIBUTES {
            Length: size_of::<OBJECT_ATTRIBUTES>() as u32,
            RootDirectory: parent.0,
            ObjectName: &object_name,
            Attributes: OBJ_CASE_INSENSITIVE,
            SecurityDescriptor: ptr::null(),
            SecurityQualityOfService: ptr::null(),
        };
        // SAFETY: IO_STATUS_BLOCK es un struct C plano; todo a cero es válido.
        let mut iosb: IO_STATUS_BLOCK = unsafe { std::mem::zeroed() };
        let mut handle: HANDLE = ptr::null_mut();
        // SAFETY: todos los punteros apuntan a datos vivos durante la llamada.
        let status = unsafe {
            NtCreateFile(
                &mut handle,
                FILE_LIST_DIRECTORY | SYNCHRONIZE,
                &attributes,
                &mut iosb,
                ptr::null(),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                FILE_OPEN,
                FILE_DIRECTORY_FILE
                    | FILE_SYNCHRONOUS_IO_NONALERT
                    | FILE_OPEN_FOR_BACKUP_INTENT
                    | FILE_OPEN_REPARSE_POINT,
                ptr::null(),
                0,
            )
        };
        if status < 0 {
            // SAFETY: función pura de conversión de códigos.
            let code = unsafe { RtlNtStatusToDosError(status) };
            return Err(io::Error::from_raw_os_error(code as i32));
        }
        Ok(DirHandle(handle))
    }

    fn list(&self, dir: &DirHandle, batch: &mut Batch<Box<[u16]>>) -> io::Result<()> {
        BUF.with_borrow_mut(|buf| {
            let mut class = self.class.load(Ordering::Relaxed);
            let mut started = false;
            loop {
                // SAFETY: el búfer está alineado a 8 bytes y mide BUF_BYTES.
                let ok =
                    unsafe { GetFileInformationByHandleEx(dir.0, class, buf.as_mut_ptr().cast(), BUF_BYTES as u32) };
                if ok == 0 {
                    // SAFETY: sin requisitos.
                    let err = unsafe { GetLastError() };
                    if err == ERROR_NO_MORE_FILES {
                        return Ok(());
                    }
                    if !started
                        && class == FileIdExtdDirectoryInfo
                        && matches!(err, ERROR_INVALID_PARAMETER | ERROR_INVALID_LEVEL | ERROR_NOT_SUPPORTED)
                    {
                        // El sistema de archivos no tiene identificadores: clase básica.
                        self.class.store(FileFullDirectoryInfo, Ordering::Relaxed);
                        class = FileFullDirectoryRestartInfo;
                        continue;
                    }
                    return Err(io::Error::from_raw_os_error(err as i32));
                }
                started = true;
                // SAFETY: el sistema acaba de escribir entradas válidas de esta clase.
                unsafe { parse_block(buf, class, batch) };
                if class == FileFullDirectoryRestartInfo {
                    class = FileFullDirectoryInfo;
                }
            }
        })
    }
}

/// Recorre las entradas de un bloque devuelto por `GetFileInformationByHandleEx`.
///
/// # Safety
/// `buf` debe contener entradas válidas de la clase `class`.
unsafe fn parse_block(buf: &[u64], class: i32, batch: &mut Batch<Box<[u16]>>) {
    let base = buf.as_ptr().cast::<u8>();
    let mut offset = 0usize;
    loop {
        // SAFETY: `offset` viene de NextEntryOffset, dentro del bloque escrito.
        let entry = unsafe { base.add(offset) };
        let next = if class == FileIdExtdDirectoryInfo {
            let e = entry.cast::<FILE_ID_EXTD_DIR_INFO>();
            // SAFETY: se leen campos sueltos sin crear referencias al struct.
            unsafe {
                let id = addr_of!((*e).FileId.Identifier).read();
                push_entry(
                    batch,
                    name_slice(addr_of!((*e).FileName).cast(), addr_of!((*e).FileNameLength).read()),
                    addr_of!((*e).FileAttributes).read(),
                    addr_of!((*e).ReparsePointTag).read(),
                    addr_of!((*e).EndOfFile).read(),
                    addr_of!((*e).AllocationSize).read(),
                    addr_of!((*e).LastWriteTime).read(),
                    file_id(id),
                );
                addr_of!((*e).NextEntryOffset).read()
            }
        } else {
            let e = entry.cast::<FILE_FULL_DIR_INFO>();
            // SAFETY: igual que arriba. En esta clase, EaSize guarda la etiqueta
            // de reparse cuando el archivo es un punto de reparse.
            unsafe {
                push_entry(
                    batch,
                    name_slice(addr_of!((*e).FileName).cast(), addr_of!((*e).FileNameLength).read()),
                    addr_of!((*e).FileAttributes).read(),
                    addr_of!((*e).EaSize).read(),
                    addr_of!((*e).EndOfFile).read(),
                    addr_of!((*e).AllocationSize).read(),
                    addr_of!((*e).LastWriteTime).read(),
                    0,
                );
                addr_of!((*e).NextEntryOffset).read()
            }
        };
        if next == 0 {
            break;
        }
        offset += next as usize;
    }
}

/// # Safety
/// `ptr` debe apuntar a `bytes` bytes válidos de UTF-16.
unsafe fn name_slice<'a>(ptr: *const u16, bytes: u32) -> &'a [u16] {
    // SAFETY: lo garantiza quien llama.
    unsafe { std::slice::from_raw_parts(ptr, bytes as usize / 2) }
}

/// Convierte el identificador de 128 bits en uno de 64. En NTFS la parte alta
/// es cero y el resultado es el número de referencia exacto.
fn file_id(raw: [u8; 16]) -> u64 {
    let lo = u64::from_le_bytes(raw[..8].try_into().unwrap());
    let hi = u64::from_le_bytes(raw[8..].try_into().unwrap());
    if hi == 0 { lo } else { lo ^ hi.wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(29) }
}

#[allow(clippy::too_many_arguments)]
fn push_entry(
    batch: &mut Batch<Box<[u16]>>,
    name: &[u16],
    attrs: u32,
    tag: u32,
    end_of_file: i64,
    allocation: i64,
    last_write: i64,
    id: u64,
) {
    const DOT: u16 = b'.' as u16;
    if name == [DOT] || name == [DOT, DOT] {
        return;
    }
    let is_dir = attrs & FILE_ATTRIBUTE_DIRECTORY != 0;
    let reparse = attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    let link = reparse && tag & REPARSE_TAG_NAME_SURROGATE != 0;

    let mut fl = 0u16;
    if is_dir {
        fl |= flags::DIR;
    }
    if reparse {
        fl |= flags::REPARSE;
    }
    if link {
        fl |= flags::LINK;
    }
    if attrs & FILE_ATTRIBUTE_HIDDEN != 0 {
        fl |= flags::HIDDEN;
    }
    if attrs & FILE_ATTRIBUTE_SYSTEM != 0 {
        fl |= flags::SYSTEM;
    }
    if attrs & FILE_ATTRIBUTE_COMPRESSED != 0 {
        fl |= flags::COMPRESSED;
    }
    if attrs & FILE_ATTRIBUTE_SPARSE_FILE != 0 {
        fl |= flags::SPARSE;
    }
    let cloud = FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS | FILE_ATTRIBUTE_RECALL_ON_OPEN | FILE_ATTRIBUTE_OFFLINE;
    if !is_dir && attrs & cloud != 0 {
        fl |= flags::CLOUD;
    }

    let name_start = batch.names.len();
    push_name(&mut batch.names, name);
    let (size, alloc) = if is_dir { (0, 0) } else { (end_of_file.max(0) as u64, allocation.max(0) as u64) };
    let key = (is_dir && !link).then(|| Box::<[u16]>::from(name));
    batch.push(name_start, size, alloc, filetime_to_unix(last_write), if is_dir { 0 } else { id }, fl, key);
}
