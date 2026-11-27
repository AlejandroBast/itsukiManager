//! Núcleo de itsukiManager.
//!
//! - [`scan`]: escaneo paralelo del disco (API nativa de Windows).
//! - [`tree`]: árbol compacto con tamaños por carpeta.
//! - [`query`]: carpetas grandes, archivos grandes, hijos ordenados.
//! - [`types`]: espacio por extensión y categoría.
//! - [`actions`]: Papelera y Explorador, con rutas protegidas.
//! - [`clean`]: basura conocida (temporales, cachés, node_modules…) y su borrado seguro.
//! - [`dupes`]: archivos duplicados.
//! - [`history`]: historial de escaneos (SQLite) y crecimiento por carpeta.
//! - [`alerts`]: avisos de disco lleno y ritmo de llenado.
//! - [`system`]: permisos y unidades.
//! - [`format`]: tamaños, cantidades y fechas en español.

pub mod actions;
pub mod alerts;
pub mod clean;
pub mod dupes;
pub mod format;
pub mod history;
pub mod query;
pub mod scan;
pub mod system;
pub mod tree;
pub mod types;

pub use clean::{JunkCategory, JunkContext, JunkItem, JunkReport, Safety, find_junk};
pub use dupes::{DupGroup, DupOptions, DupProgress, DupReport, find_duplicates};
pub use query::{DirHit, DirMode, DirQuery, FileQuery, Metric};
pub use scan::{Engine, Progress, ScanError, ScanOptions, ScanProgress, scan};
pub use tree::{Node, NodeId, ROOT, ScanStats, Tree, flags};
pub use types::{Category, TypeReport, TypeStat, type_report};
