//! Consultas sobre el árbol: carpetas grandes, archivos grandes e hijos ordenados.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use serde::{Deserialize, Serialize};

use crate::tree::{Node, NodeId, ROOT, Tree, flags};

/// Qué tamaño se usa para ordenar y filtrar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Metric {
    /// Espacio real ocupado en disco (recomendado para liberar espacio).
    #[default]
    Disk,
    /// Tamaño lógico de los archivos.
    Logical,
}

impl Metric {
    pub fn of(self, n: &Node) -> u64 {
        match self {
            Metric::Disk => n.alloc,
            Metric::Logical => n.size,
        }
    }
}

/// Qué carpetas grandes listar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DirMode {
    /// Todas las que superan el mínimo (incluye las que solo lo superan por
    /// contener otras carpetas grandes).
    #[default]
    All,
    /// Solo las "hojas": carpetas grandes sin subcarpetas grandes, o cuyos
    /// archivos directos ya superan el mínimo. Son las que de verdad ocupan.
    Leaves,
}

/// Filtro de carpetas grandes.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct DirQuery {
    /// Tamaño mínimo en bytes.
    pub min: u64,
    /// Profundidad máxima respecto a la raíz (`None` = sin límite).
    pub max_depth: Option<u32>,
    pub mode: DirMode,
    pub metric: Metric,
}

/// Carpeta encontrada por [`Tree::large_dirs`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirHit {
    pub id: NodeId,
    pub depth: u32,
}

/// Filtro de archivos grandes.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct FileQuery {
    /// Cuántos devolver como máximo.
    pub top: usize,
    /// Tamaño mínimo en bytes.
    pub min: u64,
    /// Solo archivos sin modificar desde esta fecha (segundos Unix).
    pub modified_before: Option<i64>,
    pub metric: Metric,
}

impl Tree {
    /// Carpetas que superan `q.min`, de mayor a menor.
    ///
    /// Como una carpeta nunca ocupa más que su padre, se recorre solo la parte
    /// del árbol que supera el mínimo: es casi instantáneo aunque haya
    /// millones de archivos.
    pub fn large_dirs(&self, q: &DirQuery) -> Vec<DirHit> {
        let mut out = Vec::new();
        let mut stack = vec![(ROOT, 0u32)];
        while let Some((id, depth)) = stack.pop() {
            let mut big_child = false;
            let mut direct = 0u64;
            for c in self.children(id) {
                let n = self.node(c);
                if n.is_dir() {
                    if q.metric.of(n) >= q.min {
                        big_child = true;
                        if q.max_depth.is_none_or(|m| depth < m) {
                            stack.push((c, depth + 1));
                        }
                    }
                } else if !n.has(flags::HARDLINK) {
                    direct += q.metric.of(n);
                }
            }
            if id == ROOT {
                continue;
            }
            let at_limit = q.max_depth.is_some_and(|m| depth >= m);
            let keep = match q.mode {
                DirMode::All => true,
                DirMode::Leaves => !big_child || at_limit || direct >= q.min,
            };
            if keep {
                out.push(DirHit { id, depth });
            }
        }
        out.sort_unstable_by_key(|h| (Reverse(q.metric.of(self.node(h.id))), h.id));
        out
    }

    /// Archivos más grandes, de mayor a menor.
    pub fn top_files(&self, q: &FileQuery) -> Vec<NodeId> {
        if q.top == 0 {
            return Vec::new();
        }
        let mut heap: BinaryHeap<Reverse<(u64, NodeId)>> = BinaryHeap::with_capacity(q.top + 1);
        for (i, n) in self.nodes().iter().enumerate() {
            if n.is_dir() || n.has(flags::HARDLINK) {
                continue;
            }
            let v = q.metric.of(n);
            if v == 0 || v < q.min {
                continue;
            }
            if let Some(cut) = q.modified_before {
                if n.mtime == 0 || n.mtime > cut {
                    continue;
                }
            }
            if heap.len() < q.top {
                heap.push(Reverse((v, i as NodeId)));
            } else if heap.peek().is_some_and(|Reverse((min, _))| v > *min) {
                heap.pop();
                heap.push(Reverse((v, i as NodeId)));
            }
        }
        let mut found: Vec<(u64, NodeId)> = heap.into_iter().map(|Reverse(x)| x).collect();
        found.sort_unstable_by_key(|&(v, id)| (Reverse(v), id));
        found.into_iter().map(|(_, id)| id).collect()
    }

    /// Hijos directos ordenados de mayor a menor.
    pub fn children_sorted(&self, id: NodeId, metric: Metric) -> Vec<NodeId> {
        let mut kids: Vec<NodeId> = self.children(id).collect();
        kids.sort_unstable_by_key(|&c| (Reverse(metric.of(self.node(c))), c));
        kids
    }

    /// Espacio de los archivos que están directamente en la carpeta.
    pub fn direct_files_size(&self, id: NodeId, metric: Metric) -> u64 {
        self.children(id)
            .map(|c| self.node(c))
            .filter(|n| !n.is_dir() && !n.has(flags::HARDLINK))
            .map(|n| metric.of(n))
            .sum()
    }

    /// Cantidad de subcarpetas en todo el contenido de `id`.
    pub fn count_dirs(&self, id: NodeId) -> u64 {
        let mut total = 0;
        let mut stack = vec![id];
        while let Some(d) = stack.pop() {
            for c in self.children(d) {
                if self.node(c).is_dir() {
                    total += 1;
                    stack.push(c);
                }
            }
        }
        total
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::tests::tree_from;

    const G: u64 = 1 << 30;

    /// R:\
    ///   juegos (12 G)  -> a (5 G), b (7 G)
    ///   datos  (3 G)   -> 2,5 G de archivos directos + mini (0,5 G)
    ///   suelto.iso (2 G)
    fn sample() -> Tree {
        tree_from(
            &[
                (0, "juegos", true, 0),          // 1
                (0, "datos", true, 0),           // 2
                (0, "suelto.iso", false, 2 * G), // 3
                (1, "a", true, 0),               // 4
                (1, "b", true, 0),               // 5
                (2, "mini", true, 0),            // 6
                (2, "x.bin", false, 5 * G / 2),  // 7
                (4, "a.pak", false, 5 * G),      // 8
                (5, "b.pak", false, 7 * G),      // 9
                (6, "m.bin", false, G / 2),      // 10
            ],
            None,
        )
    }

    fn names(t: &Tree, hits: &[DirHit]) -> Vec<String> {
        hits.iter().map(|h| t.name_lossy(h.id).into_owned()).collect()
    }

    #[test]
    fn carpetas_grandes_todas() {
        let t = sample();
        let q = DirQuery { min: G, max_depth: None, mode: DirMode::All, metric: Metric::Disk };
        assert_eq!(names(&t, &t.large_dirs(&q)), ["juegos", "b", "a", "datos"]);
        let q1 = DirQuery { max_depth: Some(1), ..q };
        assert_eq!(names(&t, &t.large_dirs(&q1)), ["juegos", "datos"]);
    }

    #[test]
    fn carpetas_grandes_hojas() {
        let t = sample();
        let q = DirQuery { min: G, max_depth: None, mode: DirMode::Leaves, metric: Metric::Disk };
        // "juegos" no es hoja (a y b son grandes); "datos" sí (mini es pequeña).
        assert_eq!(names(&t, &t.large_dirs(&q)), ["b", "a", "datos"]);
    }

    #[test]
    fn archivos_grandes() {
        let t = sample();
        let q = FileQuery { top: 3, min: 0, modified_before: None, metric: Metric::Disk };
        let top: Vec<_> = t.top_files(&q).iter().map(|&id| t.name_lossy(id).into_owned()).collect();
        assert_eq!(top, ["b.pak", "a.pak", "x.bin"]);
        let q = FileQuery { top: 10, min: 3 * G, ..q };
        assert_eq!(t.top_files(&q).len(), 2);
    }

    #[test]
    fn hijos_ordenados() {
        let t = sample();
        let kids: Vec<_> =
            t.children_sorted(ROOT, Metric::Disk).iter().map(|&id| t.name_lossy(id).into_owned()).collect();
        assert_eq!(kids, ["juegos", "datos", "suelto.iso"]);
        assert_eq!(t.direct_files_size(2, Metric::Disk), 5 * G / 2);
        assert_eq!(t.count_dirs(ROOT), 5);
    }
}
