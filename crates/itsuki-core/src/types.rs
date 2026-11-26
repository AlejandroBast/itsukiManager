//! Espacio por tipo de archivo (extensión y categoría).

use std::collections::HashMap;

use rayon::prelude::*;
use serde::Serialize;

use crate::tree::{Tree, flags};

/// Categoría de archivo.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub enum Category {
    Video,
    Images,
    Audio,
    Documents,
    Archives,
    Installers,
    Programs,
    DiskImages,
    GameData,
    Code,
    Databases,
    System,
    Temporary,
    Other,
    NoExtension,
}

impl Category {
    pub const ALL: [Category; 15] = [
        Category::Video,
        Category::Images,
        Category::Audio,
        Category::Documents,
        Category::Archives,
        Category::Installers,
        Category::Programs,
        Category::DiskImages,
        Category::GameData,
        Category::Code,
        Category::Databases,
        Category::System,
        Category::Temporary,
        Category::Other,
        Category::NoExtension,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::Video => "Vídeo",
            Category::Images => "Imágenes",
            Category::Audio => "Audio",
            Category::Documents => "Documentos",
            Category::Archives => "Comprimidos",
            Category::Installers => "Instaladores",
            Category::Programs => "Programas",
            Category::DiskImages => "Discos virtuales e ISO",
            Category::GameData => "Datos de juegos",
            Category::Code => "Código y desarrollo",
            Category::Databases => "Bases de datos",
            Category::System => "Sistema",
            Category::Temporary => "Temporales, caché y registros",
            Category::Other => "Otros",
            Category::NoExtension => "Sin extensión",
        }
    }

    /// Categoría de una extensión en minúsculas y sin punto.
    pub fn of_extension(ext: &str) -> Category {
        match ext {
            "mp4" | "mkv" | "avi" | "mov" | "wmv" | "flv" | "webm" | "m4v" | "mpg" | "mpeg" | "ts" | "m2ts" | "mts"
            | "3gp" | "vob" => Category::Video,
            "jpg" | "jpeg" | "png" | "gif" | "bmp" | "tif" | "tiff" | "webp" | "heic" | "heif" | "raw" | "cr2"
            | "cr3" | "nef" | "arw" | "dng" | "psd" | "svg" | "ico" | "avif" => Category::Images,
            "mp3" | "wav" | "flac" | "aac" | "ogg" | "m4a" | "wma" | "opus" | "aiff" | "mid" | "midi" => {
                Category::Audio
            }
            "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "odt" | "ods" | "odp" | "txt" | "rtf" | "md"
            | "csv" | "epub" => Category::Documents,
            "zip" | "rar" | "7z" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "lz4" | "lzma" => Category::Archives,
            "msi" | "msix" | "msixbundle" | "appx" | "appxbundle" | "msp" | "msu" => Category::Installers,
            "exe" | "dll" | "ocx" | "scr" | "com" | "node" | "jar" => Category::Programs,
            "iso" | "img" | "vhd" | "vhdx" | "vmdk" | "vdi" | "qcow2" | "wim" | "esd" | "avhdx" => Category::DiskImages,
            "pak" | "ucas" | "utoc" | "vpk" | "bsa" | "ba2" | "wad" | "bundle" | "assets" | "resource" | "forge"
            | "rpf" | "big" | "wem" | "bnk" | "sii" | "uasset" | "umap" | "upk" | "cpk" | "sb" | "sbr" => {
                Category::GameData
            }
            // ".ts" se cuenta como vídeo (MPEG-TS): en espacio pesa mucho más que TypeScript.
            "rs" | "js" | "mjs" | "cjs" | "tsx" | "jsx" | "py" | "pyc" | "java" | "class" | "c" | "cpp" | "h"
            | "hpp" | "cs" | "go" | "rb" | "php" | "html" | "css" | "scss" | "json" | "xml" | "yml" | "yaml"
            | "toml" | "lock" | "map" | "pdb" | "lib" | "obj" | "o" | "a" | "rlib" | "rmeta" | "wasm" => Category::Code,
            "db" | "sqlite" | "sqlite3" | "mdf" | "ldf" | "ndf" | "accdb" | "mdb" | "ldb" | "edb" => {
                Category::Databases
            }
            "sys" | "drv" | "cat" | "mui" | "inf" | "etl" | "evtx" | "dmp" | "manifest" | "efi" | "regtrans-ms"
            | "blf" => Category::System,
            "tmp" | "temp" | "cache" | "log" | "bak" | "old" | "part" | "crdownload" | "partial" | "dlcache" => {
                Category::Temporary
            }
            _ => Category::Other,
        }
    }
}

/// Totales de un grupo de archivos.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct TypeStat {
    pub files: u64,
    pub size: u64,
    pub alloc: u64,
}

impl TypeStat {
    fn add(&mut self, o: &TypeStat) {
        self.files += o.files;
        self.size += o.size;
        self.alloc += o.alloc;
    }
}

/// Informe de tipos, ordenado de mayor a menor espacio en disco.
#[derive(Clone, Debug, Default, Serialize)]
pub struct TypeReport {
    /// Por extensión (`""` = sin extensión).
    pub extensions: Vec<(String, TypeStat)>,
    pub categories: Vec<(Category, TypeStat)>,
}

/// Extensión de un nombre (sin el punto). Los nombres que empiezan por punto
/// (`.gitignore`) y las "extensiones" largas (nombres de caché tipo hash) no
/// cuentan como extensión.
pub(crate) fn extension_of(name: &[u8]) -> Option<&[u8]> {
    let dot = name.iter().rposition(|&b| b == b'.')?;
    if dot == 0 || dot + 1 == name.len() || name.len() - dot - 1 > 12 {
        return None;
    }
    Some(&name[dot + 1..])
}

/// Calcula el espacio por extensión y por categoría (en paralelo).
pub fn type_report(tree: &Tree) -> TypeReport {
    let per_ext: HashMap<Vec<u8>, TypeStat> = (1..tree.len() as u32)
        .into_par_iter()
        .fold(HashMap::new, |mut map: HashMap<Vec<u8>, TypeStat>, id| {
            let n = tree.node(id);
            if n.is_dir() || n.has(flags::HARDLINK) {
                return map;
            }
            let mut key = [0u8; 12];
            let ext = extension_of(tree.name_bytes(id)).unwrap_or(b"");
            for (k, b) in key.iter_mut().zip(ext) {
                *k = b.to_ascii_lowercase();
            }
            let key = &key[..ext.len()];
            let stat = TypeStat { files: 1, size: n.size, alloc: n.alloc };
            match map.get_mut(key) {
                Some(s) => s.add(&stat),
                None => {
                    map.insert(key.to_vec(), stat);
                }
            }
            map
        })
        .reduce(HashMap::new, |mut a, b| {
            for (k, v) in b {
                a.entry(k).or_default().add(&v);
            }
            a
        });

    let mut categories: HashMap<Category, TypeStat> = HashMap::new();
    let mut extensions: Vec<(String, TypeStat)> = per_ext
        .into_iter()
        .map(|(k, v)| {
            let ext = String::from_utf8_lossy(&k).into_owned();
            let cat = if ext.is_empty() { Category::NoExtension } else { Category::of_extension(&ext) };
            categories.entry(cat).or_default().add(&v);
            (ext, v)
        })
        .collect();
    extensions.sort_unstable_by(|a, b| b.1.alloc.cmp(&a.1.alloc).then_with(|| a.0.cmp(&b.0)));
    let mut categories: Vec<(Category, TypeStat)> = categories.into_iter().collect();
    categories.sort_unstable_by(|a, b| b.1.alloc.cmp(&a.1.alloc).then_with(|| a.0.cmp(&b.0)));
    TypeReport { extensions, categories }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::tests::tree_from;

    #[test]
    fn extensiones() {
        assert_eq!(extension_of(b"video.MP4"), Some(&b"MP4"[..]));
        assert_eq!(extension_of(b"archivo.tar.gz"), Some(&b"gz"[..]));
        assert_eq!(extension_of(b".gitignore"), None);
        assert_eq!(extension_of(b"sin_extension"), None);
        assert_eq!(extension_of(b"termina."), None);
        assert_eq!(extension_of(b"cache.d41d8cd98f00b204e98"), None);
    }

    #[test]
    fn informe_por_tipo() {
        let t = tree_from(
            &[(0, "a.MP4", false, 300), (0, "b.mp4", false, 200), (0, "c.zip", false, 100), (0, "LEEME", false, 50)],
            None,
        );
        let r = type_report(&t);
        assert_eq!(r.extensions[0].0, "mp4");
        assert_eq!(r.extensions[0].1, TypeStat { files: 2, size: 500, alloc: 500 });
        assert_eq!(r.categories[0].0, Category::Video);
        assert_eq!(r.categories.iter().find(|c| c.0 == Category::NoExtension).unwrap().1.files, 1);
        assert_eq!(Category::of_extension("vhdx"), Category::DiskImages);
    }
}
