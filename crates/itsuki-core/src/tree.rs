//! Árbol compacto del sistema de archivos.
//!
//! Todos los nodos viven en un único `Vec<Node>` (48 bytes por entrada) y los
//! nombres en un único búfer de bytes. Los hijos de cada carpeta son contiguos
//! y todo nodo tiene un índice mayor que el de su padre, así que los tamaños de
//! las carpetas se calculan de abajo hacia arriba en una sola pasada.

use std::borrow::Cow;
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use rayon::slice::ParallelSliceMut;
use serde::Serialize;

/// Índice de un nodo dentro del árbol.
pub type NodeId = u32;

/// Índice de la raíz del escaneo.
pub const ROOT: NodeId = 0;

/// Valor de `parent` para la raíz.
pub const NO_PARENT: NodeId = u32::MAX;

/// Banderas de cada nodo.
pub mod flags {
    /// Es una carpeta.
    pub const DIR: u16 = 1 << 0;
    /// Enlace simbólico, unión o punto de montaje: no se recorre.
    pub const LINK: u16 = 1 << 1;
    /// Atributo oculto.
    pub const HIDDEN: u16 = 1 << 2;
    /// Atributo de sistema.
    pub const SYSTEM: u16 = 1 << 3;
    /// Enlace duro repetido: su espacio ya se contó en otra ruta.
    pub const HARDLINK: u16 = 1 << 4;
    /// Carpeta que no se pudo leer (permisos).
    pub const DENIED: u16 = 1 << 5;
    /// Archivo solo en la nube (OneDrive, etc.): no ocupa disco local.
    pub const CLOUD: u16 = 1 << 6;
    /// Comprimido por NTFS.
    pub const COMPRESSED: u16 = 1 << 7;
    /// Archivo disperso.
    pub const SPARSE: u16 = 1 << 8;
    /// Tiene un punto de reanálisis (reparse point) de cualquier tipo.
    pub const REPARSE: u16 = 1 << 9;
    /// El listado de la carpeta se interrumpió a medias.
    pub const PARTIAL: u16 = 1 << 10;
}

/// Una entrada del árbol: archivo o carpeta.
///
/// En las carpetas, `size`, `alloc` y `files` son totales de todo su contenido.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct Node {
    /// Tamaño lógico en bytes (lo que "pesan" los archivos).
    pub size: u64,
    /// Espacio reservado en disco en bytes ("tamaño en disco").
    pub alloc: u64,
    /// Última modificación, en segundos Unix (0 = desconocida).
    pub mtime: i64,
    /// Carpeta padre ([`NO_PARENT`] en la raíz).
    pub parent: NodeId,
    pub(crate) name_off: u32,
    pub(crate) first: u32,
    pub(crate) len: u32,
    /// Cantidad de archivos dentro (1 en los archivos).
    pub files: u32,
    pub(crate) name_len: u16,
    /// Combinación de [`flags`].
    pub flags: u16,
}

impl Node {
    pub fn is_dir(&self) -> bool {
        self.flags & flags::DIR != 0
    }

    pub fn has(&self, flag: u16) -> bool {
        self.flags & flag != 0
    }

    /// Cantidad de hijos directos.
    pub fn child_count(&self) -> u32 {
        self.len
    }
}

/// Datos del escaneo que produjo el árbol.
#[derive(Clone, Debug, Default, Serialize)]
pub struct ScanStats {
    pub elapsed: Duration,
    pub files: u64,
    pub dirs: u64,
    /// Carpetas que no se pudieron leer.
    pub denied: u64,
    /// Enlaces duros repetidos que no se sumaron dos veces.
    pub hardlinks: u64,
    /// Motor usado: `"nativo"` o `"portable"`.
    pub engine: &'static str,
    pub threads: usize,
    /// El proceso tenía permisos de administrador.
    pub elevated: bool,
    /// Se activó el privilegio de copia de seguridad (lee carpetas protegidas).
    pub backup_privilege: bool,
}

/// Resultado de un escaneo.
pub struct Tree {
    nodes: Vec<Node>,
    names: Vec<u8>,
    root_path: PathBuf,
    denied: Vec<NodeId>,
    stats: ScanStats,
}

impl Tree {
    /// Construye el árbol final: descarta enlaces duros repetidos (si hay
    /// identificadores) y suma los tamaños de cada carpeta.
    pub(crate) fn build(
        nodes: Vec<Node>,
        names: Vec<u8>,
        ids: Option<Vec<u64>>,
        root_path: PathBuf,
        denied: Vec<NodeId>,
        mut stats: ScanStats,
    ) -> Tree {
        stats.files = nodes.iter().filter(|n| !n.is_dir()).count() as u64;
        stats.dirs = nodes.len() as u64 - stats.files;
        stats.denied = denied.len() as u64;
        let mut tree = Tree { nodes, names, root_path, denied, stats };
        if let Some(ids) = ids {
            tree.stats.hardlinks = tree.dedupe_hardlinks(&ids);
        }
        tree.aggregate();
        tree
    }

    pub(crate) fn set_elapsed(&mut self, elapsed: Duration) {
        self.stats.elapsed = elapsed;
    }

    /// Ruta absoluta que se escaneó.
    pub fn root_path(&self) -> &Path {
        &self.root_path
    }

    pub fn root(&self) -> NodeId {
        ROOT
    }

    pub fn stats(&self) -> &ScanStats {
        &self.stats
    }

    /// Cantidad total de nodos (archivos + carpetas, incluida la raíz).
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.len() <= 1
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id as usize]
    }

    /// Todos los nodos, en orden de índice.
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    /// Carpetas que no se pudieron leer.
    pub fn denied(&self) -> &[NodeId] {
        &self.denied
    }

    /// Nombre del nodo (la ruta completa en el caso de la raíz).
    pub fn name(&self, id: NodeId) -> &OsStr {
        if id == ROOT {
            return self.root_path.as_os_str();
        }
        let n = &self.nodes[id as usize];
        let start = n.name_off as usize;
        let bytes = &self.names[start..start + n.name_len as usize];
        // SAFETY: los nombres se guardan como UTF-8 válido o como bytes
        // obtenidos de `OsStr::as_encoded_bytes` en esta misma compilación.
        unsafe { OsStr::from_encoded_bytes_unchecked(bytes) }
    }

    /// Nombre para mostrar (los caracteres inválidos se sustituyen).
    pub fn name_lossy(&self, id: NodeId) -> Cow<'_, str> {
        self.name(id).to_string_lossy()
    }

    /// Bytes del nombre (UTF-8 en la práctica).
    pub(crate) fn name_bytes(&self, id: NodeId) -> &[u8] {
        let n = &self.nodes[id as usize];
        let start = n.name_off as usize;
        &self.names[start..start + n.name_len as usize]
    }

    /// Extensión en minúsculas, sin el punto (`None` si no tiene).
    pub fn extension(&self, id: NodeId) -> Option<String> {
        crate::types::extension_of(self.name_bytes(id)).map(|e| String::from_utf8_lossy(e).to_ascii_lowercase())
    }

    /// Ruta completa del nodo.
    pub fn path(&self, id: NodeId) -> PathBuf {
        let mut chain = Vec::with_capacity(16);
        let mut cur = id;
        while cur != ROOT {
            chain.push(cur);
            cur = self.nodes[cur as usize].parent;
        }
        let mut path = self.root_path.clone();
        for &c in chain.iter().rev() {
            path.push(self.name(c));
        }
        path
    }

    /// Padre del nodo (`None` en la raíz).
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        let p = self.nodes[id as usize].parent;
        (p != NO_PARENT).then_some(p)
    }

    /// Profundidad respecto a la raíz (la raíz es 0).
    pub fn depth(&self, id: NodeId) -> u32 {
        let mut depth = 0;
        let mut cur = id;
        while cur != ROOT {
            cur = self.nodes[cur as usize].parent;
            depth += 1;
        }
        depth
    }

    /// Hijos directos de una carpeta, en el orden en que se leyeron.
    pub fn children(&self, id: NodeId) -> impl ExactSizeIterator<Item = NodeId> + use<> {
        let n = &self.nodes[id as usize];
        n.first..n.first + n.len
    }

    /// Busca un nodo por su ruta (sin distinguir mayúsculas en ASCII).
    pub fn find(&self, path: &Path) -> Option<NodeId> {
        let rel = path.strip_prefix(&self.root_path).ok()?;
        let mut cur = ROOT;
        for comp in rel.components() {
            let Component::Normal(part) = comp else { continue };
            let want = part.as_encoded_bytes();
            cur = self
                .children(cur)
                .find(|&c| self.name_bytes(c) == want)
                .or_else(|| self.children(cur).find(|&c| self.name_bytes(c).eq_ignore_ascii_case(want)))?;
        }
        Some(cur)
    }

    /// Memoria aproximada que ocupa el árbol, en bytes.
    pub fn memory_bytes(&self) -> usize {
        self.nodes.capacity() * size_of::<Node>() + self.names.capacity() + self.denied.capacity() * 4
    }

    /// Marca los enlaces duros repetidos para no contarlos dos veces.
    ///
    /// De cada grupo se conserva la ruta menor en orden alfabético (así el
    /// resultado no depende del orden del escaneo) y se le asigna el mayor
    /// tamaño visto, porque NTFS solo actualiza el tamaño de la entrada usada
    /// para modificar el archivo.
    fn dedupe_hardlinks(&mut self, ids: &[u64]) -> u64 {
        let mut pairs: Vec<(u64, NodeId)> = ids
            .iter()
            .enumerate()
            .filter(|&(i, &id)| id != 0 && !self.nodes[i].is_dir())
            .map(|(i, &id)| (id, i as NodeId))
            .collect();
        if pairs.len() < 2 {
            return 0;
        }
        pairs.par_sort_unstable();
        let mut repeated = 0;
        for group in pairs.chunk_by(|a, b| a.0 == b.0) {
            if group.len() < 2 {
                continue;
            }
            let keeper =
                group.iter().map(|&(_, id)| (self.path(id), id)).min().map(|(_, id)| id).expect("grupo no vacío");
            let size = group.iter().map(|&(_, id)| self.nodes[id as usize].size).max().unwrap_or(0);
            let alloc = group.iter().map(|&(_, id)| self.nodes[id as usize].alloc).max().unwrap_or(0);
            for &(_, id) in group {
                let n = &mut self.nodes[id as usize];
                if id == keeper {
                    n.size = size;
                    n.alloc = alloc;
                } else {
                    n.flags |= flags::HARDLINK;
                    repeated += 1;
                }
            }
        }
        repeated
    }

    /// Suma tamaños y archivos de abajo hacia arriba.
    fn aggregate(&mut self) {
        let nodes = &mut self.nodes;
        for n in nodes.iter_mut().filter(|n| n.is_dir()) {
            n.size = 0;
            n.alloc = 0;
            n.files = 0;
        }
        for i in (1..nodes.len()).rev() {
            let n = nodes[i];
            let (size, alloc, files) = if n.is_dir() {
                (n.size, n.alloc, n.files)
            } else if n.has(flags::HARDLINK) {
                (0, 0, 1)
            } else {
                (n.size, n.alloc, 1)
            };
            let p = &mut nodes[n.parent as usize];
            p.size += size;
            p.alloc += alloc;
            p.files += files;
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Construye un árbol a mano: `(padre, nombre, es_carpeta, tamaño)`.
    /// Los padres deben aparecer antes que sus hijos y los hijos de cada
    /// carpeta deben ir seguidos.
    pub(crate) fn tree_from(entries: &[(NodeId, &str, bool, u64)], ids: Option<Vec<u64>>) -> Tree {
        let mut nodes = vec![Node { parent: NO_PARENT, flags: flags::DIR, ..Default::default() }];
        let mut names = Vec::new();
        for &(parent, name, dir, size) in entries {
            let idx = nodes.len() as u32;
            let p = &mut nodes[parent as usize];
            if p.len == 0 {
                p.first = idx;
            }
            p.len += 1;
            nodes.push(Node {
                size,
                alloc: size,
                parent,
                name_off: names.len() as u32,
                name_len: name.len() as u16,
                flags: if dir { flags::DIR } else { 0 },
                ..Default::default()
            });
            names.extend_from_slice(name.as_bytes());
        }
        Tree::build(nodes, names, ids, PathBuf::from("R:\\"), Vec::new(), ScanStats::default())
    }

    #[test]
    fn suma_de_abajo_hacia_arriba() {
        let t = tree_from(
            &[(0, "a", true, 0), (0, "f1", false, 10), (1, "b", true, 0), (1, "f2", false, 20), (3, "f3", false, 30)],
            None,
        );
        assert_eq!(t.node(ROOT).size, 60);
        assert_eq!(t.node(ROOT).files, 3);
        assert_eq!(t.node(1).size, 50);
        assert_eq!(t.node(3).size, 30);
        assert_eq!(t.path(5), PathBuf::from("R:\\a\\b\\f3"));
        assert_eq!(t.depth(5), 3);
        assert_eq!(t.find(Path::new("R:\\A\\b\\F3")), Some(5));
        assert_eq!(t.stats().files, 3);
        assert_eq!(t.stats().dirs, 3);
    }

    #[test]
    fn enlaces_duros_cuentan_una_vez() {
        // f1 (en "x") y f2 (en "y") son el mismo archivo.
        let ids = vec![0, 0, 0, 7, 7];
        let t =
            tree_from(&[(0, "x", true, 0), (0, "y", true, 0), (1, "f1", false, 100), (2, "f2", false, 100)], Some(ids));
        assert_eq!(t.node(ROOT).size, 100);
        assert_eq!(t.node(1).size, 100, "se conserva la ruta menor (x)");
        assert_eq!(t.node(2).size, 0);
        assert!(t.node(4).has(flags::HARDLINK));
        assert_eq!(t.stats().hardlinks, 1);
    }
}
