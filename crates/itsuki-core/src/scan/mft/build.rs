//! Construcción del árbol a partir de los registros de la MFT.
//!
//! La MFT es una tabla plana: cada nombre apunta a su carpeta padre. Aquí se
//! agrupan los nombres por padre (orden CSR) y se recorre en anchura desde la
//! carpeta pedida, así los hijos de cada carpeta quedan contiguos y todo
//! nodo tiene un índice mayor que su padre, como espera [`crate::tree::Tree`].

use super::parse::{ExtPart, IN_USE, IS_DIR, Link, RecInfo};
use crate::scan::util::REPARSE_TAG_NAME_SURROGATE;
use crate::scan::walk::WalkOutput;
use crate::tree::{NO_PARENT, Node, flags};

/// Une a cada registro base los datos de sus registros de extensión.
pub(crate) fn apply_extensions(recs: &mut [RecInfo], ext: &[ExtPart]) {
    for e in ext {
        let Some(r) = recs.get_mut(e.base as usize) else { continue };
        r.alloc += e.alloc;
        if let Some(size) = e.size {
            r.size = size;
        }
        // La etiqueta de una extensión viene de $REPARSE_POINT: es la fiable.
        if e.tag != 0 {
            r.tag = e.tag;
        }
    }
}

/// Construye los nodos del subárbol de `root_frn`.
pub(crate) fn build(recs: &[RecInfo], links: &[Link], names: Vec<u8>, root_frn: u32, keep_ids: bool) -> WalkOutput {
    let n = recs.len();
    let flags_of = |frn: u32| recs.get(frn as usize).map_or(0, |r| r.flags);
    let valid = |l: &Link| {
        l.frn != l.parent && flags_of(l.frn) & IN_USE != 0 && flags_of(l.parent) & (IN_USE | IS_DIR) == IN_USE | IS_DIR
    };

    // Hijos agrupados por carpeta padre.
    let mut start = vec![0u32; n + 1];
    let mut link_count = vec![0u8; n];
    for l in links.iter().filter(|l| valid(l)) {
        start[l.parent as usize + 1] += 1;
        let c = &mut link_count[l.frn as usize];
        *c = c.saturating_add(1);
    }
    for i in 1..=n {
        start[i] += start[i - 1];
    }
    let mut fill = start.clone();
    let mut order = vec![0u32; start[n] as usize];
    for (i, l) in links.iter().enumerate().filter(|(_, l)| valid(l)) {
        let slot = &mut fill[l.parent as usize];
        order[*slot as usize] = i as u32;
        *slot += 1;
    }

    let mut nodes = Vec::with_capacity(order.len() + 1);
    let mut ids = keep_ids.then(|| Vec::with_capacity(order.len() + 1));
    let root = recs.get(root_frn as usize).copied().unwrap_or_default();
    nodes.push(Node {
        alloc: root.alloc,
        mtime: root.mtime,
        parent: NO_PARENT,
        flags: node_flags(&root) | flags::DIR,
        ..Default::default()
    });
    if let Some(ids) = ids.as_mut() {
        ids.push(0);
    }

    // Recorrido en anchura. `visited` evita ciclos si la MFT está dañada.
    let mut visited = vec![false; n];
    if let Some(v) = visited.get_mut(root_frn as usize) {
        *v = true;
    }
    let mut queue: Vec<(u32, u32)> = vec![(root_frn, 0)];
    let mut head = 0;
    while head < queue.len() {
        let (frn, idx) = queue[head];
        head += 1;
        let Some(range) = start.get(frn as usize..frn as usize + 2) else { continue };
        let first = nodes.len() as u32;
        for &li in &order[range[0] as usize..range[1] as usize] {
            let l = links[li as usize];
            let r = &recs[l.frn as usize];
            let is_dir = r.flags & IS_DIR != 0;
            if is_dir {
                if visited[l.frn as usize] {
                    continue;
                }
                visited[l.frn as usize] = true;
                queue.push((l.frn, nodes.len() as u32));
            }
            nodes.push(Node {
                size: if is_dir { 0 } else { r.size },
                alloc: r.alloc,
                mtime: r.mtime,
                parent: idx,
                name_off: l.name_off,
                name_len: l.name_len,
                flags: node_flags(r),
                ..Default::default()
            });
            if let Some(ids) = ids.as_mut() {
                let shared = !is_dir && link_count[l.frn as usize] > 1;
                ids.push(if shared { u64::from(l.frn) + 1 } else { 0 });
            }
        }
        let len = nodes.len() as u32 - first;
        let d = &mut nodes[idx as usize];
        d.first = first;
        d.len = len;
    }
    WalkOutput { nodes, names, ids, denied: Vec::new() }
}

/// Banderas del árbol a partir de los atributos de Windows del registro.
fn node_flags(r: &RecInfo) -> u16 {
    const HIDDEN: u32 = 0x2;
    const SYSTEM: u32 = 0x4;
    const SPARSE: u32 = 0x200;
    const REPARSE: u32 = 0x400;
    const COMPRESSED: u32 = 0x800;
    const OFFLINE: u32 = 0x1000;
    const RECALL_ON_OPEN: u32 = 0x4_0000;
    const RECALL_ON_DATA_ACCESS: u32 = 0x40_0000;

    let a = r.attrs;
    let is_dir = r.flags & IS_DIR != 0;
    let mut f = 0;
    if is_dir {
        f |= flags::DIR;
    }
    if a & HIDDEN != 0 {
        f |= flags::HIDDEN;
    }
    if a & SYSTEM != 0 {
        f |= flags::SYSTEM;
    }
    if a & SPARSE != 0 {
        f |= flags::SPARSE;
    }
    if a & COMPRESSED != 0 {
        f |= flags::COMPRESSED;
    }
    if a & REPARSE != 0 {
        f |= flags::REPARSE;
        if r.tag & REPARSE_TAG_NAME_SURROGATE != 0 {
            f |= flags::LINK;
        }
    }
    if !is_dir && a & (OFFLINE | RECALL_ON_OPEN | RECALL_ON_DATA_ACCESS) != 0 {
        f |= flags::CLOUD;
    }
    f
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::tree::{ROOT, ScanStats, Tree};

    struct Fake {
        recs: Vec<RecInfo>,
        links: Vec<Link>,
        names: Vec<u8>,
    }

    impl Fake {
        fn new(n: usize) -> Fake {
            let mut f = Fake { recs: vec![RecInfo::default(); n], links: Vec::new(), names: Vec::new() };
            f.dir(5, 5, "."); // la raíz es su propio padre
            f
        }
        fn rec(&mut self, frn: u32, flags: u8, size: u64) {
            self.recs[frn as usize] = RecInfo { size, alloc: size, flags, ..Default::default() };
        }
        fn name(&mut self, frn: u32, parent: u32, name: &str) {
            let off = self.names.len() as u32;
            self.names.extend_from_slice(name.as_bytes());
            self.links.push(Link { frn, parent, name_off: off, name_len: name.len() as u16 });
        }
        fn dir(&mut self, frn: u32, parent: u32, name: &str) {
            self.rec(frn, IN_USE | IS_DIR, 0);
            self.name(frn, parent, name);
        }
        fn file(&mut self, frn: u32, parent: u32, name: &str, size: u64) {
            self.rec(frn, IN_USE, size);
            self.name(frn, parent, name);
        }
        fn tree(self, root: u32) -> Tree {
            let out = build(&self.recs, &self.links, self.names, root, true);
            Tree::build(out.nodes, out.names, out.ids, PathBuf::from("C:\\"), out.denied, ScanStats::default())
        }
    }

    #[test]
    fn arbol_desde_la_mft() {
        let mut f = Fake::new(40);
        f.dir(30, 5, "Juegos");
        f.dir(31, 30, "Steam");
        f.file(32, 31, "juego.pak", 1000);
        f.file(33, 5, "pagefile.sys", 500);
        f.file(34, 99, "huerfano.txt", 7); // padre inexistente: no aparece
        f.file(35, 32, "dentro-de-archivo", 9); // padre no es carpeta: no aparece
        f.rec(36, 0, 0); // registro libre
        f.name(36, 5, "borrado.txt");
        let t = f.tree(5);

        assert_eq!(t.node(ROOT).size, 1500);
        assert_eq!(t.node(ROOT).files, 2);
        assert_eq!(t.len(), 5, "raíz, Juegos, pagefile.sys, Steam, juego.pak");
        let pak = t.find(&PathBuf::from("C:\\Juegos\\Steam\\juego.pak")).unwrap();
        assert_eq!(t.node(pak).size, 1000);
        assert_eq!(t.node(t.find(&PathBuf::from("C:\\Juegos")).unwrap()).size, 1000);
        for id in 1..t.len() as u32 {
            assert!(t.node(id).parent < id, "el padre va antes que el hijo");
        }
    }

    #[test]
    fn enlaces_duros_y_subcarpeta() {
        let mut f = Fake::new(40);
        f.dir(30, 5, "a");
        f.dir(31, 5, "b");
        f.file(32, 30, "grande.bin", 4000);
        f.name(32, 31, "mismo.bin"); // segundo nombre del mismo archivo
        let t = f.tree(5);
        assert_eq!(t.node(ROOT).size, 4000, "el enlace duro cuenta una vez");
        assert_eq!(t.stats().hardlinks, 1);

        let mut f = Fake::new(40);
        f.dir(30, 5, "a");
        f.dir(31, 30, "sub");
        f.file(32, 31, "x.bin", 10);
        f.file(33, 5, "fuera.bin", 99);
        let t = f.tree(30);
        assert_eq!(t.node(ROOT).size, 10, "solo el subárbol pedido");
        assert_eq!(t.len(), 3);
    }

    #[test]
    fn ciclos_no_cuelgan() {
        let mut f = Fake::new(40);
        f.dir(30, 31, "a");
        f.dir(31, 30, "b"); // a y b son padre uno del otro (MFT dañada)
        f.name(30, 5, "a");
        let t = f.tree(5);
        assert!(t.len() <= 3);
    }

    #[test]
    fn extensiones_y_banderas() {
        let mut recs = vec![RecInfo { flags: IN_USE, attrs: 0x400 | 0x2, ..Default::default() }; 3];
        apply_extensions(&mut recs, &[ExtPart { base: 1, size: Some(77), alloc: 4096, tag: 0xA000_000C }]);
        assert_eq!((recs[1].size, recs[1].alloc, recs[1].tag), (77, 4096, 0xA000_000C));
        let fl = node_flags(&recs[1]);
        assert!(fl & flags::LINK != 0 && fl & flags::HIDDEN != 0 && fl & flags::REPARSE != 0);
        let cloud = RecInfo { flags: IN_USE, attrs: 0x40_0000, ..Default::default() };
        assert!(node_flags(&cloud) & flags::CLOUD != 0);
    }
}
