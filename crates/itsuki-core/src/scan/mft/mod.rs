//! Motor MFT: lee directamente la tabla maestra de archivos de NTFS.
//!
//! En vez de abrir cada carpeta (lo que paga el coste del antivirus y del
//! kernel por cada una), se abre el volumen una sola vez y se lee la MFT con
//! lecturas grandes y secuenciales. Necesita permisos de administrador y solo
//! funciona en volúmenes NTFS locales.
//!
//! Ventajas además de la velocidad: ve todas las carpetas (también las
//! protegidas), los tamaños salen del atributo de datos (siempre exactos) y
//! cuenta el espacio de flujos alternativos e índices de carpeta.

mod build;
mod parse;
#[cfg(windows)]
mod volume;

#[cfg(windows)]
pub(crate) use volume::scan;
