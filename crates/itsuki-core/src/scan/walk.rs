//! Recorrido paralelo genérico.
//!
//! Cada carpeta es una tarea de rayon: se lista, sus entradas se copian de una
//! sola vez al árbol compartido (los hijos quedan contiguos) y cada subcarpeta
//! se lanza como una tarea nueva. El robo de trabajo de rayon reparte las ramas
//! grandes entre todos los hilos.

use std::io;
use std::path::Path;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, Mutex, MutexGuard};

use super::ScanProgress;
use crate::tree::{NO_PARENT, Node, NodeId, flags};

/// Entrada leída de una carpeta, todavía sin insertar en el árbol.
pub(crate) struct RawEntry {
    pub size: u64,
    pub alloc: u64,
    pub mtime: i64,
    /// Identificador de archivo para detectar enlaces duros (0 = sin identificador).
    pub id: u64,
    pub name_off: u32,
    pub name_len: u16,
    pub flags: u16,
}

/// Contenido de una carpeta tal como lo entrega un backend.
pub(crate) struct Batch<K> {
    pub entries: Vec<RawEntry>,
    pub names: Vec<u8>,
    /// Subcarpetas a recorrer: (índice en `entries`, clave para abrirlas).
    pub subdirs: Vec<(u32, K)>,
}

impl<K> Batch<K> {
    fn new() -> Self {
        Batch { entries: Vec::with_capacity(32), names: Vec::with_capacity(1024), subdirs: Vec::new() }
    }

    /// Registra una entrada cuyo nombre ya se escribió en `names` desde `name_start`.
    #[allow(clippy::too_many_arguments)]
    pub fn push(&mut self, name_start: usize, size: u64, alloc: u64, mtime: i64, id: u64, flags: u16, key: Option<K>) {
        let index = self.entries.len() as u32;
        let name_len = (self.names.len() - name_start).min(u16::MAX as usize) as u16;
        self.entries.push(RawEntry { size, alloc, mtime, id, name_off: name_start as u32, name_len, flags });
        if let Some(key) = key {
            self.subdirs.push((index, key));
        }
    }
}

/// Forma de abrir y listar carpetas (API nativa de Windows o `std::fs`).
pub(crate) trait Backend: Sync {
    /// Carpeta abierta (handle o ruta).
    type Dir: Send + Sync + 'static;
    /// Lo necesario para abrir una subcarpeta desde su padre.
    type Key: Send + 'static;

    fn open_root(&self, path: &Path) -> io::Result<Self::Dir>;
    fn open_child(&self, parent: &Self::Dir, key: &Self::Key) -> io::Result<Self::Dir>;
    fn list(&self, dir: &Self::Dir, batch: &mut Batch<Self::Key>) -> io::Result<()>;
}

/// Árbol en construcción, compartido entre hilos.
struct Arena {
    nodes: Vec<Node>,
    names: Vec<u8>,
    ids: Vec<u64>,
    denied: Vec<NodeId>,
}

impl Arena {
    /// Copia el contenido de una carpeta y devuelve el índice de su primer hijo.
    fn commit<K>(&mut self, dir: NodeId, batch: &Batch<K>, failed: bool, keep_ids: bool) -> u32 {
        let first = self.nodes.len() as u32;
        let name_base = self.names.len() as u32;
        self.names.extend_from_slice(&batch.names);
        self.nodes.extend(batch.entries.iter().map(|e| Node {
            size: e.size,
            alloc: e.alloc,
            mtime: e.mtime,
            parent: dir,
            name_off: name_base + e.name_off,
            name_len: e.name_len,
            flags: e.flags,
            ..Default::default()
        }));
        if keep_ids {
            self.ids.extend(batch.entries.iter().map(|e| e.id));
        }
        let d = &mut self.nodes[dir as usize];
        d.first = first;
        d.len = batch.entries.len() as u32;
        if failed {
            if batch.entries.is_empty() {
                d.flags |= flags::DENIED;
                self.denied.push(dir);
            } else {
                d.flags |= flags::PARTIAL;
            }
        }
        first
    }

    fn mark_denied(&mut self, dir: NodeId) {
        self.nodes[dir as usize].flags |= flags::DENIED;
        self.denied.push(dir);
    }
}

struct Ctx<'a, B: Backend> {
    backend: &'a B,
    arena: Mutex<Arena>,
    progress: &'a ScanProgress,
    keep_ids: bool,
}

impl<B: Backend> Ctx<'_, B> {
    fn arena(&self) -> MutexGuard<'_, Arena> {
        // Si un hilo entró en pánico el árbol sigue siendo coherente.
        self.arena.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Resultado crudo del recorrido.
pub(crate) struct WalkOutput {
    pub nodes: Vec<Node>,
    pub names: Vec<u8>,
    pub ids: Option<Vec<u64>>,
    pub denied: Vec<NodeId>,
}

/// Recorre `root` en paralelo usando `pool`.
pub(crate) fn run<B: Backend>(
    backend: &B,
    root: &Path,
    pool: &rayon::ThreadPool,
    progress: &ScanProgress,
    keep_ids: bool,
) -> io::Result<WalkOutput> {
    let root_dir = backend.open_root(root)?;
    let mut nodes = Vec::with_capacity(1 << 16);
    nodes.push(Node { parent: NO_PARENT, flags: flags::DIR, ..Default::default() });
    let ctx = Ctx {
        backend,
        arena: Mutex::new(Arena {
            nodes,
            names: Vec::with_capacity(1 << 20),
            ids: if keep_ids { vec![0] } else { Vec::new() },
            denied: Vec::new(),
        }),
        progress,
        keep_ids,
    };
    pool.install(|| rayon::scope(|s| visit(&ctx, s, 0, root_dir)));
    let arena = ctx.arena.into_inner().unwrap_or_else(|e| e.into_inner());
    Ok(WalkOutput { nodes: arena.nodes, names: arena.names, ids: keep_ids.then_some(arena.ids), denied: arena.denied })
}

fn visit<'s, B: Backend>(ctx: &'s Ctx<'s, B>, scope: &rayon::Scope<'s>, node: NodeId, dir: B::Dir) {
    if ctx.progress.is_cancelled() {
        return;
    }
    let mut batch = Batch::new();
    let listed = ctx.backend.list(&dir, &mut batch);

    let (mut files, mut bytes) = (0u64, 0u64);
    for e in batch.entries.iter().filter(|e| e.flags & flags::DIR == 0) {
        files += 1;
        bytes += e.alloc;
    }
    let first = ctx.arena().commit(node, &batch, listed.is_err(), ctx.keep_ids);

    let p = ctx.progress;
    p.files.fetch_add(files, Relaxed);
    p.bytes.fetch_add(bytes, Relaxed);
    p.dirs.fetch_add(1, Relaxed);
    if listed.is_err() {
        p.errors.fetch_add(1, Relaxed);
    }
    if batch.subdirs.is_empty() {
        return;
    }

    // El handle del padre vive mientras queden hijos por abrir.
    let dir = Arc::new(dir);
    for (local, key) in batch.subdirs {
        let child = first + local;
        let parent = Arc::clone(&dir);
        scope.spawn(move |s| {
            if ctx.progress.is_cancelled() {
                return;
            }
            let opened = ctx.backend.open_child(&parent, &key);
            drop(parent);
            match opened {
                Ok(d) => visit(ctx, s, child, d),
                Err(_) => {
                    ctx.progress.errors.fetch_add(1, Relaxed);
                    ctx.arena().mark_denied(child);
                }
            }
        });
    }
}
