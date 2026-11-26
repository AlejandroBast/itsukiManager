//! Pruebas del escaneo sobre carpetas reales creadas en %TEMP%.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use itsuki_core::{Engine, Metric, ROOT, ScanError, ScanOptions, ScanProgress, Tree, flags, scan};

/// Carpeta temporal que se borra al terminar la prueba.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let p = std::env::temp_dir().join(format!("itsuki-prueba-{name}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }

    fn file(&self, rel: &str, bytes: usize) -> PathBuf {
        let p = self.0.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, vec![7u8; bytes]).unwrap();
        p
    }

    fn dir(&self, rel: &str) -> PathBuf {
        let p = self.0.join(rel);
        fs::create_dir_all(&p).unwrap();
        p
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn engines() -> Vec<Engine> {
    if cfg!(windows) { vec![Engine::Native, Engine::Portable] } else { vec![Engine::Portable] }
}

fn scan_with(root: &Path, engine: Engine, dedupe: bool) -> Tree {
    let opts = ScanOptions { engine, dedupe_hardlinks: dedupe, threads: 4 };
    scan(root, &opts, &ScanProgress::new()).unwrap()
}

fn find(tree: &Tree, root: &Path, rel: &str) -> itsuki_core::NodeId {
    tree.find(&root.join(rel)).unwrap_or_else(|| panic!("no se encontró {rel}"))
}

#[test]
fn cuenta_tamanos_y_archivos() {
    let t = TempDir::new("tamanos");
    t.file("a.bin", 1000);
    t.file("sub/b.bin", 2000);
    t.file("sub/deep/c.bin", 3000);
    t.dir("vacia");

    for engine in engines() {
        let tree = scan_with(&t.0, engine, true);
        let root = tree.node(ROOT);
        assert_eq!(root.size, 6000, "{engine:?}");
        assert!(root.alloc >= 6000 || cfg!(not(windows)), "{engine:?}");
        assert_eq!(root.files, 3, "{engine:?}");
        assert_eq!(tree.children(ROOT).len(), 3, "{engine:?}");

        let sub = find(&tree, &t.0, "sub");
        assert!(tree.node(sub).is_dir());
        assert_eq!(tree.node(sub).size, 5000);
        assert_eq!(tree.node(sub).files, 2);

        let c = find(&tree, &t.0, "sub/deep/c.bin");
        assert_eq!(tree.path(c), t.0.join("sub").join("deep").join("c.bin"));
        assert_eq!(tree.depth(c), 3);
        assert!(tree.node(c).mtime > 1_600_000_000, "fecha de modificación");

        let vacia = find(&tree, &t.0, "vacia");
        assert_eq!(tree.node(vacia).size, 0);
        assert_eq!(tree.stats().files, 3);
        assert_eq!(tree.stats().dirs, 4);
        assert_eq!(tree.stats().denied, 0);
    }
}

#[test]
fn motores_coinciden() {
    let t = TempDir::new("motores");
    for i in 0..40 {
        t.file(&format!("d{}/e{}/f{i}.dat", i % 5, i % 3), 100 + i * 37);
    }
    t.file("Ñandú 日本語 😀/archivo con espacios.txt", 4321);
    if !cfg!(windows) {
        return;
    }
    let native = scan_with(&t.0, Engine::Native, true);
    let portable = scan_with(&t.0, Engine::Portable, true);
    assert_eq!(native.node(ROOT).size, portable.node(ROOT).size);
    assert_eq!(native.node(ROOT).files, portable.node(ROOT).files);
    assert_eq!(native.len(), portable.len());

    let id = find(&native, &t.0, "Ñandú 日本語 😀/archivo con espacios.txt");
    assert_eq!(native.name_lossy(id), "archivo con espacios.txt");
    assert!(fs::metadata(native.path(id)).is_ok(), "la ruta reconstruida existe");
}

#[test]
fn enlaces_duros_se_cuentan_una_vez() {
    let t = TempDir::new("enlaces-duros");
    let original = t.file("x/grande.bin", 100_000);
    t.dir("y");
    fs::hard_link(&original, t.0.join("y").join("copia.bin")).unwrap();

    let engine = if cfg!(windows) { Engine::Native } else { Engine::Portable };
    let tree = scan_with(&t.0, engine, true);
    assert_eq!(tree.node(ROOT).size, 100_000);
    assert_eq!(tree.stats().hardlinks, 1);
    let copia = find(&tree, &t.0, "y/copia.bin");
    assert!(tree.node(copia).has(flags::HARDLINK), "se conserva x/, la ruta menor");
    assert_eq!(tree.node(find(&tree, &t.0, "x")).size, 100_000);

    let sin = scan_with(&t.0, engine, false);
    assert_eq!(sin.node(ROOT).size, 200_000);
}

#[cfg(windows)]
#[test]
fn no_sigue_uniones() {
    let t = TempDir::new("uniones");
    let real = t.dir("real");
    t.file("real/datos.bin", 5000);
    let union = t.0.join("union");
    let ok = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&union)
        .arg(&real)
        .output()
        .unwrap()
        .status
        .success();
    assert!(ok, "no se pudo crear la unión");

    for engine in engines() {
        let tree = scan_with(&t.0, engine, true);
        assert_eq!(tree.node(ROOT).size, 5000, "{engine:?}: la unión no se cuenta dos veces");
        let u = find(&tree, &t.0, "union");
        assert!(tree.node(u).has(flags::LINK), "{engine:?}");
        assert_eq!(tree.node(u).child_count(), 0, "{engine:?}");
    }
}

#[test]
fn consultas_sobre_escaneo_real() {
    let t = TempDir::new("consultas");
    t.file("grande/a.bin", 300_000);
    t.file("grande/b.bin", 200_000);
    t.file("pequena/c.bin", 10_000);

    let tree = scan_with(&t.0, Engine::Auto, true);
    let q = itsuki_core::DirQuery {
        min: 100_000,
        max_depth: None,
        mode: itsuki_core::DirMode::All,
        metric: Metric::Logical,
    };
    let dirs = tree.large_dirs(&q);
    assert_eq!(dirs.len(), 1);
    assert_eq!(tree.name_lossy(dirs[0].id), "grande");

    let fq = itsuki_core::FileQuery { top: 2, min: 0, modified_before: None, metric: Metric::Logical };
    let top: Vec<_> = tree.top_files(&fq).iter().map(|&id| tree.name_lossy(id).into_owned()).collect();
    assert_eq!(top, ["a.bin", "b.bin"]);

    let report = itsuki_core::type_report(&tree);
    assert_eq!(report.extensions[0].0, "bin");
    assert_eq!(report.extensions[0].1.files, 3);
}

#[test]
fn cancelar_detiene_el_escaneo() {
    let t = TempDir::new("cancelar");
    t.file("a/b.bin", 10);
    let progress = ScanProgress::new();
    progress.cancel();
    let r = scan(&t.0, &ScanOptions::default(), &progress);
    assert!(matches!(r, Err(ScanError::Cancelled)));
}

#[test]
fn errores_de_ruta() {
    let t = TempDir::new("errores");
    let archivo = t.file("solo.txt", 5);
    assert!(matches!(scan(&archivo, &ScanOptions::default(), &ScanProgress::new()), Err(ScanError::NotADirectory(_))));
    let falta = t.0.join("no-existe");
    assert!(matches!(scan(&falta, &ScanOptions::default(), &ScanProgress::new()), Err(ScanError::Io { .. })));
}

/// Compara el motor MFT con el nativo. Solo se ejecuta como administrador.
#[cfg(windows)]
#[test]
fn mft_coincide_con_nativo() {
    use std::io::Write;
    if !itsuki_core::system::is_elevated() {
        eprintln!("mft_coincide_con_nativo omitida: requiere administrador");
        return;
    }
    let t = TempDir::new("mft");
    // Se fuerza la escritura a disco: el motor MFT lee el volumen directamente.
    let write_synced = |rel: &str, bytes: usize| {
        let p = t.0.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        let mut f = fs::File::create(&p).unwrap();
        f.write_all(&vec![3u8; bytes]).unwrap();
        f.sync_all().unwrap();
        p
    };
    let original = write_synced("a/grande.bin", 300_000);
    write_synced("a/sub/b.bin", 20_000);
    write_synced("c/ñandú 日本.txt", 1234);
    fs::hard_link(&original, t.0.join("c").join("enlace.bin")).unwrap();
    let ok = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(t.0.join("union"))
        .arg(t.0.join("a"))
        .output()
        .unwrap()
        .status
        .success();
    assert!(ok);
    let expected = 300_000 + 20_000 + 1234;

    // NTFS escribe los registros de la MFT en segundo plano: se reintenta unos segundos.
    let deadline = Instant::now() + Duration::from_secs(30);
    let mft = loop {
        let tree = scan_with(&t.0, Engine::Mft, true);
        if tree.node(ROOT).size == expected && tree.stats().files == 4 || Instant::now() > deadline {
            break tree;
        }
        std::thread::sleep(Duration::from_millis(500));
    };
    let native = scan_with(&t.0, Engine::Native, true);
    assert_eq!(mft.stats().engine, "mft");
    assert_eq!(mft.node(ROOT).size, expected);
    assert_eq!(mft.node(ROOT).size, native.node(ROOT).size);
    assert_eq!(mft.stats().files, native.stats().files);
    assert_eq!(mft.stats().hardlinks, 1);
    let u = find(&mft, &t.0, "union");
    assert!(mft.node(u).has(flags::LINK));
    assert_eq!(mft.node(u).child_count(), 0);
    let txt = find(&mft, &t.0, "c/ñandú 日本.txt");
    assert_eq!(mft.node(txt).size, 1234);
    assert!(fs::metadata(mft.path(txt)).is_ok());
}

/// Sin permisos de administrador el motor MFT avisa en vez de fallar raro.
#[cfg(windows)]
#[test]
fn mft_sin_admin_da_error_claro() {
    if itsuki_core::system::is_elevated() {
        return;
    }
    let t = TempDir::new("mft-sin-admin");
    let opts = ScanOptions { engine: Engine::Mft, ..Default::default() };
    assert!(matches!(scan(&t.0, &opts, &ScanProgress::new()), Err(ScanError::NeedsAdmin)));
}
