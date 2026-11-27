//! Pruebas de limpieza y duplicados con archivos reales en %TEMP%.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use itsuki_core::clean::delete::junk_path_allowed;
use itsuki_core::clean::{Action, JunkCategory, JunkContext, Location, Safety, clean_item, find_junk};
use itsuki_core::{DupOptions, DupProgress, ScanOptions, ScanProgress, Tree, find_duplicates, scan};

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        // target\tmp de cargo: fuera de AppData, donde los patrones de proyectos sí se aplican.
        let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
        let p = base.join(format!("itsuki-prueba-{name}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }

    fn file(&self, rel: &str, bytes: usize) -> PathBuf {
        self.write(rel, &vec![9u8; bytes])
    }

    fn write(&self, rel: &str, data: &[u8]) -> PathBuf {
        let p = self.0.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, data).unwrap();
        p
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn scan_all(root: &Path) -> Tree {
    scan(root, &ScanOptions::default(), &ScanProgress::new()).unwrap()
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
}

/// Cambia la fecha de modificación (también de carpetas).
fn set_mtime(path: &Path, when: SystemTime) {
    let mut opts = fs::File::options();
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        opts.access_mode(0x100).custom_flags(0x0200_0000); // FILE_WRITE_ATTRIBUTES + BACKUP_SEMANTICS
    }
    #[cfg(not(windows))]
    opts.write(true);
    opts.open(path).unwrap().set_modified(when).unwrap();
}

fn temp_location(path: PathBuf, min_age: i64) -> Location {
    Location {
        path,
        category: JunkCategory::Temp,
        safety: Safety::Safe,
        action: Action::Contents { min_age },
        label: "Temporales de prueba",
        needs_admin: false,
    }
}

#[test]
fn temporales_respetan_la_antiguedad() {
    let t = TempDir::new("limpieza-temp");
    let three_days_ago = SystemTime::now() - Duration::from_secs(3 * 86_400);
    let old = t.file("Temp/viejo.log", 4000);
    set_mtime(&old, three_days_ago);
    let new = t.file("Temp/nuevo.log", 5000);
    t.file("Temp/carpeta_vieja/dentro.bin", 6000);
    set_mtime(&t.0.join("Temp").join("carpeta_vieja"), three_days_ago);

    let tree = scan_all(&t.0);
    let mut ctx = JunkContext::empty(now());
    ctx.locations.push(temp_location(t.0.join("Temp"), 86_400));
    let report = find_junk(&tree, &ctx);
    let item = report.items.iter().find(|i| i.label == "Temporales de prueba").expect("se detecta Temp");
    assert_eq!(item.files, 2, "solo lo que tiene más de un día");

    let done = clean_item(&tree, item, false).unwrap();
    assert_eq!((done.files, done.bytes, done.skipped), (2, 10_000, 0));
    assert!(!old.exists());
    assert!(!t.0.join("Temp").join("carpeta_vieja").exists());
    assert!(new.exists(), "lo reciente se conserva");
    assert!(t.0.join("Temp").exists(), "la carpeta Temp se conserva");
}

#[cfg(windows)]
#[test]
fn limpiar_no_sigue_uniones() {
    let t = TempDir::new("limpieza-union");
    let precious = t.file("valioso/importante.txt", 1234);
    t.file("Temp/basura.tmp", 100);
    let link = t.0.join("Temp").join("enlace");
    let ok = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(t.0.join("valioso"))
        .output()
        .unwrap()
        .status
        .success();
    assert!(ok);

    let tree = scan_all(&t.0);
    let mut ctx = JunkContext::empty(now());
    ctx.locations.push(temp_location(t.0.join("Temp"), 0));
    let report = find_junk(&tree, &ctx);
    let item = report.items.iter().find(|i| i.label == "Temporales de prueba").unwrap();
    clean_item(&tree, item, false).unwrap();

    assert!(precious.exists(), "lo que hay detrás de la unión no se toca");
    assert!(fs::symlink_metadata(&link).is_err(), "la unión sí se borra");
    assert!(!t.0.join("Temp").join("basura.tmp").exists());
}

#[test]
fn node_modules_se_borra_entero() {
    let t = TempDir::new("limpieza-proyecto");
    t.file("app/package.json", 10);
    t.file("app/node_modules/lib/index.js", 3000);
    t.file("app/node_modules/lib/package.json", 20);
    t.file("app/src/main.js", 50);

    let tree = scan_all(&t.0);
    let report = find_junk(&tree, &JunkContext::empty(now()));
    assert_eq!(report.items.len(), 1, "el node_modules anidado no se cuenta aparte");
    let item = &report.items[0];
    assert_eq!(
        (item.label, item.category, item.safety),
        ("node_modules", JunkCategory::DevDependencies, Safety::Regenerable)
    );

    let done = clean_item(&tree, item, false).unwrap();
    assert_eq!(done.files, 2);
    assert!(!t.0.join("app").join("node_modules").exists());
    assert!(t.0.join("app").join("package.json").exists());
    assert!(t.0.join("app").join("src").join("main.js").exists());
}

#[cfg(windows)]
#[test]
fn rutas_permitidas_para_limpiar() {
    let windows = PathBuf::from(std::env::var("SystemRoot").unwrap());
    let profile = PathBuf::from(std::env::var("USERPROFILE").unwrap());
    assert!(!junk_path_allowed(Path::new("C:\\")));
    assert!(!junk_path_allowed(&windows));
    assert!(!junk_path_allowed(&windows.join("System32")));
    assert!(!junk_path_allowed(&profile));
    assert!(!junk_path_allowed(&profile.join("Documents")));
    assert!(junk_path_allowed(&windows.join("Temp").join("x.tmp")));
    assert!(junk_path_allowed(&windows.join("SoftwareDistribution").join("Download").join("abc")));
    assert!(junk_path_allowed(&std::env::temp_dir().join("x.tmp")));
}

#[test]
fn duplicados_por_contenido() {
    let t = TempDir::new("duplicados");
    let data: Vec<u8> = (0..300_000u32).map(|i| (i * 7 % 251) as u8).collect();
    let a = t.write("a.bin", &data);
    t.write("sub/b.bin", &data);
    let mut last = data.clone();
    *last.last_mut().unwrap() ^= 1;
    t.write("c.bin", &last); // distinto al final: lo separa el hash parcial
    let mut middle = data.clone();
    middle[150_000] ^= 1;
    t.write("d.bin", &middle); // distinto en el medio: lo separa el hash completo
    fs::hard_link(&a, t.0.join("e.bin")).unwrap(); // mismo archivo: no es un duplicado
    t.write("f.bin", &data[..500]);
    t.write("g.bin", &data[..500]); // por debajo del mínimo

    let tree = scan_all(&t.0);
    let opts = DupOptions { min_size: 1000, include_system: true, include_roots: Vec::new(), threads: 2 };
    let report = find_duplicates(&tree, &opts, &DupProgress::new()).unwrap();
    assert_eq!(report.groups.len(), 1);
    let names: Vec<String> = report.groups[0].files.iter().map(|&id| tree.name_lossy(id).into_owned()).collect();
    assert_eq!(names, ["a.bin", "b.bin"]);
    assert_eq!(report.groups[0].size, 300_000);
    assert!(report.wasted() >= 300_000);
    assert_eq!(report.errors, 0);
}
