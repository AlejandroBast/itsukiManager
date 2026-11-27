//! Historial de escaneos en SQLite.
//!
//! Cada foto guarda los totales del escaneo, el espacio de la unidad y el
//! tamaño de las carpetas grandes (100 MB o más). Comparando dos fotos se ve
//! qué carpetas crecieron o se redujeron, y con la evolución del espacio
//! libre se calcula a qué ritmo se llena cada unidad.
//!
//! La base de datos está en `%LOCALAPPDATA%\itsukiManager\historial.db`
//! (o en la carpeta de la variable `ITSUKI_DATOS`).

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::query::{DirMode, DirQuery, Metric};
use crate::system::Volume;
use crate::tree::{ROOT, Tree};

/// Tamaño mínimo de las carpetas que se guardan en cada foto.
pub const DEFAULT_MIN_FOLDER: u64 = 100 << 20;
/// Fotos que se conservan por cada ruta escaneada.
pub const DEFAULT_KEEP: usize = 100;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS snapshots (
    id           INTEGER PRIMARY KEY,
    taken_at     INTEGER NOT NULL,
    root         TEXT    NOT NULL,
    root_key     TEXT    NOT NULL,
    engine       TEXT    NOT NULL,
    alloc        INTEGER NOT NULL,
    size         INTEGER NOT NULL,
    files        INTEGER NOT NULL,
    dirs         INTEGER NOT NULL,
    volume_key   TEXT,
    volume_total INTEGER,
    volume_free  INTEGER,
    elapsed_ms   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS snapshots_root ON snapshots (root_key, taken_at);
CREATE INDEX IF NOT EXISTS snapshots_volume ON snapshots (volume_key, taken_at);
CREATE TABLE IF NOT EXISTS folders (
    snapshot INTEGER NOT NULL REFERENCES snapshots (id) ON DELETE CASCADE,
    path     TEXT    NOT NULL,
    alloc    INTEGER NOT NULL,
    size     INTEGER NOT NULL,
    files    INTEGER NOT NULL,
    PRIMARY KEY (snapshot, path)
) WITHOUT ROWID;
PRAGMA user_version = 1;
";

/// Error del historial.
#[derive(Debug)]
pub enum HistoryError {
    Db(rusqlite::Error),
    Io(std::io::Error),
}

impl fmt::Display for HistoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HistoryError::Db(e) => write!(f, "base de datos del historial: {e}"),
            HistoryError::Io(e) => write!(f, "carpeta del historial: {e}"),
        }
    }
}

impl std::error::Error for HistoryError {}

impl From<rusqlite::Error> for HistoryError {
    fn from(e: rusqlite::Error) -> Self {
        HistoryError::Db(e)
    }
}

impl From<std::io::Error> for HistoryError {
    fn from(e: std::io::Error) -> Self {
        HistoryError::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, HistoryError>;

/// Una foto del historial.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Snapshot {
    pub id: i64,
    /// Momento de la foto (segundos Unix).
    pub taken_at: i64,
    pub root: String,
    pub engine: String,
    pub alloc: u64,
    pub size: u64,
    pub files: u64,
    pub dirs: u64,
    pub volume_total: Option<u64>,
    pub volume_free: Option<u64>,
    pub elapsed_ms: u64,
}

impl Snapshot {
    pub fn volume_used(&self) -> Option<u64> {
        Some(self.volume_total?.saturating_sub(self.volume_free?))
    }
}

/// Tamaño de una carpeta en una foto.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FolderRow {
    pub path: String,
    pub alloc: u64,
    pub size: u64,
    pub files: u64,
}

/// Cambio de una carpeta entre dos fotos. `None` significa que no estaba (o
/// que era menor que el mínimo que se guarda).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Change {
    pub path: String,
    pub before: Option<u64>,
    pub after: Option<u64>,
}

impl Change {
    /// Diferencia de espacio en disco (positiva si creció).
    pub fn delta(&self) -> i64 {
        self.after.unwrap_or(0) as i64 - self.before.unwrap_or(0) as i64
    }
}

/// Clave de una ruta para comparar sin distinguir mayúsculas.
pub fn root_key(path: &Path) -> String {
    let mut s = path.to_string_lossy().to_lowercase();
    while s.ends_with('\\') && !(s.len() == 3 && s.as_bytes()[1] == b':') {
        s.pop();
    }
    s
}

/// Carpetas de `tree` que se guardan en una foto: la raíz y todas las de al
/// menos `min` bytes en disco.
pub fn folder_rows(tree: &Tree, min: u64) -> Vec<FolderRow> {
    let row = |id| {
        let n = tree.node(id);
        FolderRow { path: tree.path(id).display().to_string(), alloc: n.alloc, size: n.size, files: u64::from(n.files) }
    };
    let mut rows = vec![row(ROOT)];
    let q = DirQuery { min, max_depth: None, mode: DirMode::All, metric: Metric::Disk };
    rows.extend(tree.large_dirs(&q).into_iter().map(|h| row(h.id)));
    rows
}

/// Historial guardado en una base de datos SQLite.
pub struct History {
    conn: Connection,
}

impl History {
    /// Carpeta de datos de itsukiManager.
    pub fn default_dir() -> PathBuf {
        if let Some(dir) = std::env::var_os("ITSUKI_DATOS") {
            return PathBuf::from(dir);
        }
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("itsukiManager")
    }

    /// Abre (o crea) el historial en su ubicación por defecto.
    pub fn open_default() -> Result<History> {
        let dir = Self::default_dir();
        std::fs::create_dir_all(&dir)?;
        Self::open(&dir.join("historial.db"))
    }

    pub fn open(path: &Path) -> Result<History> {
        Self::init(Connection::open(path)?)
    }

    /// Historial en memoria (para pruebas).
    pub fn open_in_memory() -> Result<History> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<History> {
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        conn.execute_batch(SCHEMA)?;
        Ok(History { conn })
    }

    /// Guarda una foto del árbol y devuelve su número. Conserva solo las
    /// [`DEFAULT_KEEP`] fotos más recientes de cada ruta.
    pub fn save(&mut self, tree: &Tree, volume: Option<&Volume>, min_folder: u64, taken_at: i64) -> Result<i64> {
        let root = tree.root_path();
        let r = tree.node(ROOT);
        let s = tree.stats();
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO snapshots (taken_at, root, root_key, engine, alloc, size, files, dirs, volume_key,
                                    volume_total, volume_free, elapsed_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                taken_at,
                root.display().to_string(),
                root_key(root),
                s.engine,
                r.alloc as i64,
                r.size as i64,
                s.files as i64,
                s.dirs as i64,
                volume.map(|v| root_key(&v.root)),
                volume.map(|v| v.total as i64),
                volume.map(|v| v.free as i64),
                s.elapsed.as_millis() as i64,
            ],
        )?;
        let id = tx.last_insert_rowid();
        {
            let mut insert = tx.prepare(
                "INSERT OR IGNORE INTO folders (snapshot, path, alloc, size, files) VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for f in folder_rows(tree, min_folder) {
                insert.execute(params![id, f.path, f.alloc as i64, f.size as i64, f.files as i64])?;
            }
        }
        tx.commit()?;
        self.prune(&root_key(root), DEFAULT_KEEP)?;
        Ok(id)
    }

    /// Fotos guardadas, de la más reciente a la más antigua (todas o solo las de `root`).
    pub fn list(&self, root: Option<&Path>) -> Result<Vec<Snapshot>> {
        const COLS: &str =
            "id, taken_at, root, engine, alloc, size, files, dirs, volume_total, volume_free, elapsed_ms";
        let rows = match root {
            Some(r) => {
                let mut stmt = self.conn.prepare(&format!(
                    "SELECT {COLS} FROM snapshots WHERE root_key = ?1 ORDER BY taken_at DESC, id DESC"
                ))?;
                stmt.query_map([root_key(r)], snapshot_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?
            }
            None => {
                let mut stmt =
                    self.conn.prepare(&format!("SELECT {COLS} FROM snapshots ORDER BY taken_at DESC, id DESC"))?;
                stmt.query_map([], snapshot_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?
            }
        };
        Ok(rows)
    }

    pub fn get(&self, id: i64) -> Result<Option<Snapshot>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, taken_at, root, engine, alloc, size, files, dirs, volume_total, volume_free, elapsed_ms
                 FROM snapshots WHERE id = ?1",
                [id],
                snapshot_from_row,
            )
            .optional()?)
    }

    /// Carpetas guardadas en una foto.
    pub fn folders(&self, id: i64) -> Result<Vec<FolderRow>> {
        let mut stmt = self.conn.prepare("SELECT path, alloc, size, files FROM folders WHERE snapshot = ?1")?;
        let rows = stmt
            .query_map([id], |r| {
                Ok(FolderRow {
                    path: r.get(0)?,
                    alloc: r.get::<_, i64>(1)? as u64,
                    size: r.get::<_, i64>(2)? as u64,
                    files: r.get::<_, i64>(3)? as u64,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Borra una foto. Devuelve `false` si no existía.
    pub fn delete(&mut self, id: i64) -> Result<bool> {
        Ok(self.conn.execute("DELETE FROM snapshots WHERE id = ?1", [id])? > 0)
    }

    /// Deja solo las `keep` fotos más recientes de una ruta. Devuelve cuántas borró.
    pub fn prune(&mut self, root_key: &str, keep: usize) -> Result<usize> {
        Ok(self.conn.execute(
            "DELETE FROM snapshots WHERE root_key = ?1 AND id NOT IN (
                 SELECT id FROM snapshots WHERE root_key = ?1 ORDER BY taken_at DESC, id DESC LIMIT ?2)",
            params![root_key, keep as i64],
        )?)
    }

    /// Foto más reciente de `root` tomada como muy tarde en `before`, sin contar `exclude`.
    pub fn latest_before(&self, root: &Path, before: i64, exclude: Option<i64>) -> Result<Option<Snapshot>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, taken_at, root, engine, alloc, size, files, dirs, volume_total, volume_free, elapsed_ms
                 FROM snapshots WHERE root_key = ?1 AND taken_at <= ?2 AND id <> ?3
                 ORDER BY taken_at DESC, id DESC LIMIT 1",
                params![root_key(root), before, exclude.unwrap_or(-1)],
                snapshot_from_row,
            )
            .optional()?)
    }

    /// Foto más antigua tomada en la unidad `volume_root` (de cualquier carpeta)
    /// dentro de la ventana `[from, to]`. Sirve para medir el ritmo de llenado.
    pub fn oldest_on_volume(&self, volume_root: &Path, from: i64, to: i64) -> Result<Option<Snapshot>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, taken_at, root, engine, alloc, size, files, dirs, volume_total, volume_free, elapsed_ms
                 FROM snapshots WHERE volume_key = ?1 AND volume_free IS NOT NULL AND taken_at BETWEEN ?2 AND ?3
                 ORDER BY taken_at ASC, id ASC LIMIT 1",
                params![root_key(volume_root), from, to],
                snapshot_from_row,
            )
            .optional()?)
    }
}

fn snapshot_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Snapshot> {
    Ok(Snapshot {
        id: r.get(0)?,
        taken_at: r.get(1)?,
        root: r.get(2)?,
        engine: r.get(3)?,
        alloc: r.get::<_, i64>(4)? as u64,
        size: r.get::<_, i64>(5)? as u64,
        files: r.get::<_, i64>(6)? as u64,
        dirs: r.get::<_, i64>(7)? as u64,
        volume_total: r.get::<_, Option<i64>>(8)?.map(|v| v as u64),
        volume_free: r.get::<_, Option<i64>>(9)?.map(|v| v as u64),
        elapsed_ms: r.get::<_, i64>(10)? as u64,
    })
}

/// Carpeta padre de una clave de ruta (`c:\a\b` → `c:\a`, `c:\a` → `c:\`).
fn parent_key(key: &str) -> Option<&str> {
    let pos = key.rfind('\\')?;
    if pos == 2 && key.as_bytes()[1] == b':' { (key.len() > 3).then(|| &key[..3]) } else { Some(&key[..pos]) }
}

/// Cambios por carpeta entre dos fotos, de mayor a menor (en valor absoluto),
/// a partir de `min_change` bytes.
///
/// Con `leaves`, se quitan las carpetas cuyo cambio se explica casi entero
/// (80 % o más) por una de sus subcarpetas: así se ve dónde está de verdad
/// el crecimiento y no toda la cadena de carpetas padre.
pub fn compare(old: &[FolderRow], new: &[FolderRow], min_change: u64, leaves: bool) -> Vec<Change> {
    let mut map: HashMap<String, Change> = HashMap::with_capacity(old.len() + new.len());
    for f in old {
        map.insert(f.path.to_lowercase(), Change { path: f.path.clone(), before: Some(f.alloc), after: None });
    }
    for f in new {
        map.entry(f.path.to_lowercase())
            .and_modify(|c| {
                c.after = Some(f.alloc);
                c.path = f.path.clone();
            })
            .or_insert_with(|| Change { path: f.path.clone(), before: None, after: Some(f.alloc) });
    }

    let mut changes: Vec<(String, Change)> =
        map.into_iter().filter(|(_, c)| c.delta().unsigned_abs() >= min_change.max(1)).collect();
    if leaves {
        // Mayor cambio de cada carpeta entre sus hijas directas, por signo.
        let mut best_child: HashMap<String, (i64, i64)> = HashMap::new();
        for (key, c) in &changes {
            if let Some(parent) = parent_key(key) {
                let e = best_child.entry(parent.to_string()).or_insert((0, 0));
                let d = c.delta();
                e.0 = e.0.max(d);
                e.1 = e.1.min(d);
            }
        }
        changes.retain(|(key, c)| {
            let d = c.delta();
            let Some(&(up, down)) = best_child.get(key) else { return true };
            let child = if d > 0 { up } else { down };
            // child < 80 % de d, en enteros para no depender del redondeo.
            u128::from(child.unsigned_abs()) * 5 < u128::from(d.unsigned_abs()) * 4
        });
    }
    let mut out: Vec<Change> = changes.into_iter().map(|(_, c)| c).collect();
    out.sort_by(|a, b| b.delta().unsigned_abs().cmp(&a.delta().unsigned_abs()).then_with(|| a.path.cmp(&b.path)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::tests::tree_from;

    const MB: u64 = 1 << 20;

    fn rows(t: &Tree) -> Vec<FolderRow> {
        folder_rows(t, 100 * MB)
    }

    #[test]
    fn guarda_y_lista_fotos() {
        let mut h = History::open_in_memory().unwrap();
        let t = tree_from(
            &[(0, "grande", true, 0), (0, "chica", true, 0), (1, "a", false, 300 * MB), (2, "b", false, MB)],
            None,
        );
        let vol = Volume {
            root: PathBuf::from("R:\\"),
            label: String::new(),
            filesystem: "NTFS".into(),
            total: 1000 * MB,
            free: 600 * MB,
            kind: crate::system::DriveKind::Fixed,
            seek_penalty: Some(false),
        };
        let id = h.save(&t, Some(&vol), 100 * MB, 1_000).unwrap();
        let list = h.list(Some(Path::new("r:\\"))).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, id);
        assert_eq!(list[0].alloc, 301 * MB);
        assert_eq!(list[0].volume_used(), Some(400 * MB));
        let folders = h.folders(id).unwrap();
        let mut names: Vec<_> = folders.iter().map(|f| f.path.as_str()).collect();
        names.sort();
        assert_eq!(names, ["R:\\", "R:\\grande"], "las carpetas pequeñas no se guardan");
        assert!(h.delete(id).unwrap());
        assert!(h.folders(id).unwrap().is_empty(), "las carpetas se borran con su foto");
    }

    #[test]
    fn conserva_las_mas_recientes_y_busca_por_fecha() {
        let mut h = History::open_in_memory().unwrap();
        let t = tree_from(&[(0, "x", false, MB)], None);
        let ids: Vec<i64> = (0..5).map(|i| h.save(&t, None, 100 * MB, 1_000 + i * 100).unwrap()).collect();
        assert_eq!(h.prune("r:\\", 3).unwrap(), 2);
        let left: Vec<i64> = h.list(None).unwrap().iter().map(|s| s.id).collect();
        assert_eq!(left, vec![ids[4], ids[3], ids[2]]);
        let r = Path::new("R:\\");
        assert_eq!(h.latest_before(r, 1_250, None).unwrap().unwrap().id, ids[2]);
        assert_eq!(h.latest_before(r, 2_000, Some(ids[4])).unwrap().unwrap().id, ids[3]);
        assert!(h.latest_before(r, 500, None).unwrap().is_none());
        assert!(h.oldest_on_volume(r, 1_000, 2_000).unwrap().is_none(), "sin datos de la unidad");
    }

    #[test]
    fn crecimiento_muestra_la_carpeta_responsable() {
        let before = tree_from(
            &[
                (0, "Users", true, 0),
                (1, "ana", true, 0),
                (2, "Docker", true, 0),
                (2, "Fotos", true, 0),
                (3, "d.vhdx", false, 2000 * MB),
                (4, "f", false, 500 * MB),
            ],
            None,
        );
        let after = tree_from(
            &[
                (0, "Users", true, 0),
                (1, "ana", true, 0),
                (2, "Docker", true, 0),
                (2, "Fotos", true, 0),
                (2, "Juego", true, 0),
                (3, "d.vhdx", false, 5000 * MB),
                (4, "f", false, 450 * MB),
                (5, "j", false, 800 * MB),
            ],
            None,
        );
        let all = compare(&rows(&before), &rows(&after), 10 * MB, false);
        assert_eq!(all[0].path, "R:\\", "sin filtrar aparecen también los padres");

        let leaves = compare(&rows(&before), &rows(&after), 10 * MB, true);
        let got: Vec<(&str, i64)> = leaves.iter().map(|c| (c.path.as_str(), c.delta() / MB as i64)).collect();
        assert_eq!(
            got,
            vec![("R:\\Users\\ana\\Docker", 3000), ("R:\\Users\\ana\\Juego", 800), ("R:\\Users\\ana\\Fotos", -50)]
        );
        assert_eq!(leaves[1].before, None, "carpeta nueva");
    }

    #[test]
    fn claves_de_ruta() {
        assert_eq!(root_key(Path::new("C:\\")), "c:\\");
        assert_eq!(root_key(Path::new("C:\\Users\\")), "c:\\users");
        assert_eq!(parent_key("c:\\users\\ana"), Some("c:\\users"));
        assert_eq!(parent_key("c:\\users"), Some("c:\\"));
        assert_eq!(parent_key("c:\\"), None);
    }
}
