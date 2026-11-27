//! Reglas de detección de basura.

use super::{Action, JunkCategory as C, JunkContext, JunkItem, JunkReport, Safety as S};
use crate::tree::{NodeId, ROOT, Tree, flags};

const GB: u64 = 1 << 30;

const ADVICE_HIBERFIL: &str = "Reduce la hibernación con «powercfg /h /type reduced» o desactívala con «powercfg /h off» \
     (terminal de administrador).";
const ADVICE_WINDOWS_OLD: &str =
    "Bórrala con «Liberar espacio en disco» → «Limpiar archivos del sistema» → «Instalación anterior de Windows».";
const ADVICE_RESTORE: &str = "Son puntos de restauración: ajusta su espacio en «Protección del sistema» o con \
     «vssadmin resize shadowstorage».";
const ADVICE_WINSXS: &str =
    "Limpia componentes antiguos con «Dism /Online /Cleanup-Image /StartComponentCleanup» (administrador).";
const ADVICE_DOCKER: &str = "Borra imágenes y volúmenes sin uso con «docker system prune -a» y compacta el disco: \
     «wsl --shutdown» y luego «Optimize-VHD» o diskpart «compact vdisk».";
const ADVICE_WSL: &str =
    "Compacta el disco de WSL: «wsl --shutdown» y «wsl --manage <distro> --set-sparse true» u «Optimize-VHD».";

/// Partes de ruta (en minúsculas) de los navegadores conocidos.
const BROWSERS: [&str; 10] = [
    "\\google\\chrome\\",
    "\\microsoft\\edge\\",
    "\\bravesoftware\\",
    "\\opera software\\",
    "\\vivaldi\\",
    "\\chromium\\",
    "\\yandex\\",
    "\\mozilla\\",
    "\\prismaaccessbrowser\\",
    "\\thebrowsercompany\\",
];

/// Nombres de carpetas de caché propios de Chromium/Electron.
const CHROMIUM_CACHES: [&str; 8] = [
    "code cache",
    "gpucache",
    "dawncache",
    "dawngraphitecache",
    "dawnwebgpucache",
    "graphitedawncache",
    "grshadercache",
    "shadercache",
];

/// Partes de ruta donde viven programas instalados: los patrones de proyectos
/// (node_modules, target, __pycache__…) no se aplican ahí, porque esas
/// carpetas forman parte del programa y borrarlas lo rompería.
const APP_PLACES: [&str; 7] =
    ["\\appdata\\", "\\.cache\\", "\\.vscode\\", "\\.cargo\\", "\\.rustup\\", "\\.codex\\", "\\resources\\app"];

/// Carpetas del sistema y de programas (en minúsculas), desde las variables de entorno.
fn system_prefixes() -> Vec<String> {
    ["SystemRoot", "ProgramFiles", "ProgramFiles(x86)", "ProgramW6432", "ProgramData"]
        .iter()
        .filter_map(std::env::var_os)
        .map(|v| format!("{}\\", v.to_string_lossy().trim_end_matches('\\').to_lowercase()))
        .collect()
}

fn in_app_place(lower_path: &str, prefixes: &[String]) -> bool {
    prefixes.iter().any(|p| lower_path.starts_with(p.as_str())) || APP_PLACES.iter().any(|p| lower_path.contains(p))
}

const INSTALLER_EXTS: [&str; 9] = ["exe", "msi", "msix", "msixbundle", "appx", "iso", "zip", "7z", "rar"];

fn eq(a: &[u8], b: &str) -> bool {
    a.eq_ignore_ascii_case(b.as_bytes())
}

/// Busca basura en `tree`.
pub fn find_junk(tree: &Tree, ctx: &JunkContext) -> JunkReport {
    let n = tree.len();
    let mut covered = vec![false; n];
    let mut items = Vec::new();
    let prefixes = system_prefixes();
    let is_covered = |covered: &[bool], mut id: NodeId| loop {
        if covered[id as usize] {
            return true;
        }
        match tree.parent(id) {
            Some(p) => id = p,
            None => return false,
        }
    };

    // 1. Ubicaciones fijas.
    for loc in &ctx.locations {
        let Some(id) = tree.find(&loc.path) else { continue };
        let wants_file = loc.action == Action::Files;
        if tree.node(id).is_dir() == wants_file || id == ROOT || is_covered(&covered, id) {
            continue;
        }
        let (bytes, files) = measure(tree, id, &loc.action, ctx.now);
        if bytes == 0 || bytes < ctx.min_item {
            continue;
        }
        covered[id as usize] = true;
        items.push(JunkItem {
            category: loc.category,
            safety: loc.safety,
            action: loc.action.clone(),
            label: loc.label,
            nodes: vec![id],
            bytes,
            files,
            needs_admin: loc.needs_admin,
        });
    }

    // 2. Elementos de la raíz de una unidad: Papelera y sugerencias del sistema.
    let at_drive_root = tree.root_path().parent().is_none();
    if at_drive_root {
        for c in tree.children(ROOT) {
            let name = tree.name_bytes(c);
            let node = tree.node(c);
            let advice = |text: &'static str, label: &'static str| JunkItem {
                category: C::Advice,
                safety: S::Advice,
                action: Action::Advice(text),
                label,
                nodes: vec![c],
                bytes: node.alloc,
                files: u64::from(node.files.max(1)),
                needs_admin: true,
            };
            if node.is_dir() && eq(name, "$Recycle.Bin") && node.alloc > 0 {
                covered[c as usize] = true;
                items.push(JunkItem {
                    category: C::RecycleBin,
                    safety: S::Review,
                    action: Action::EmptyRecycleBin,
                    label: "Papelera de reciclaje",
                    nodes: vec![c],
                    bytes: node.alloc,
                    files: u64::from(node.files),
                    needs_admin: false,
                });
            } else if !node.is_dir() && eq(name, "hiberfil.sys") && node.alloc >= GB {
                items.push(advice(ADVICE_HIBERFIL, "Archivo de hibernación"));
            } else if node.is_dir() && (eq(name, "Windows.old") || eq(name, "$Windows.~BT") || eq(name, "$Windows.~WS"))
            {
                covered[c as usize] = true;
                items.push(advice(ADVICE_WINDOWS_OLD, "Instalación anterior de Windows"));
            } else if node.is_dir() && eq(name, "System Volume Information") && node.alloc >= 5 * GB {
                covered[c as usize] = true;
                items.push(advice(ADVICE_RESTORE, "Puntos de restauración"));
            }
        }
    }
    if let Some(winsxs) = ctx.windows.as_ref().and_then(|w| tree.find(&w.join("WinSxS"))) {
        if tree.node(winsxs).alloc >= 8 * GB && !is_covered(&covered, winsxs) {
            let node = tree.node(winsxs);
            items.push(JunkItem {
                category: C::Advice,
                safety: S::Advice,
                action: Action::Advice(ADVICE_WINSXS),
                label: "Almacén de componentes de Windows (WinSxS)",
                nodes: vec![winsxs],
                bytes: node.alloc,
                files: u64::from(node.files),
                needs_admin: true,
            });
        }
    }

    // 3. Instaladores y comprimidos antiguos en Descargas.
    if let Some(dl) = ctx.downloads.as_ref().and_then(|d| tree.find(d)) {
        let old: Vec<NodeId> = tree
            .children(dl)
            .filter(|&c| {
                let f = tree.node(c);
                !f.is_dir()
                    && !f.has(flags::HARDLINK)
                    && f.alloc >= ctx.downloads_min_size
                    && f.mtime > 0
                    && ctx.now - f.mtime >= ctx.downloads_min_age
                    && tree.extension(c).is_some_and(|e| INSTALLER_EXTS.contains(&e.as_str()))
            })
            .collect();
        if !old.is_empty() {
            let bytes = old.iter().map(|&c| tree.node(c).alloc).sum();
            items.push(JunkItem {
                category: C::OldDownloads,
                safety: S::Review,
                action: Action::Files,
                label: "Instaladores y comprimidos de hace más de 30 días",
                files: old.len() as u64,
                nodes: old,
                bytes,
                needs_admin: false,
            });
        }
    }

    // 4. Patrones en todo el árbol (los padres van siempre antes que los hijos).
    for id in 1..n as NodeId {
        let node = tree.node(id);
        if covered[node.parent as usize] {
            covered[id as usize] = true;
            continue;
        }
        if covered[id as usize] || node.has(flags::LINK) {
            continue;
        }
        if !node.is_dir() {
            if let Some(item) = virtual_disk_advice(tree, id) {
                items.push(item);
            }
            continue;
        }
        let Some((category, safety, action, label)) = match_dir(tree, id, &prefixes) else { continue };
        let (bytes, files) = measure(tree, id, &action, ctx.now);
        if bytes == 0 || bytes < ctx.min_item {
            continue;
        }
        covered[id as usize] = true;
        items.push(JunkItem { category, safety, action, label, nodes: vec![id], bytes, files, needs_admin: false });
    }

    items.sort_by(|a, b| a.category.cmp(&b.category).then(b.bytes.cmp(&a.bytes)));
    JunkReport { items }
}

/// Espacio y archivos que liberaría una acción.
fn measure(tree: &Tree, id: NodeId, action: &Action, now: i64) -> (u64, u64) {
    let node = tree.node(id);
    match action {
        Action::Contents { min_age } => {
            let mut bytes = 0;
            let mut files = 0;
            for c in tree.children(id) {
                let child = tree.node(c);
                if *min_age > 0 && child.mtime > 0 && now - child.mtime < *min_age {
                    continue;
                }
                if child.is_dir() {
                    bytes += child.alloc;
                    files += u64::from(child.files);
                } else if !child.has(flags::HARDLINK) {
                    bytes += child.alloc;
                    files += 1;
                }
            }
            (bytes, files)
        }
        Action::Files => (if node.has(flags::HARDLINK) { 0 } else { node.alloc }, 1),
        _ => (node.alloc, u64::from(node.files)),
    }
}

/// Reconoce carpetas de basura por su nombre y su entorno.
fn match_dir(tree: &Tree, id: NodeId, prefixes: &[String]) -> Option<(C, S, Action, &'static str)> {
    let name = tree.name_bytes(id);
    let parent = tree.node(id).parent;
    let pname = tree.name_bytes(parent);
    let sibling = |want_dir: bool, n: &str| {
        tree.children(parent).any(|c| tree.node(c).is_dir() == want_dir && eq(tree.name_bytes(c), n))
    };
    let sibling_ext = |exts: &[&str]| {
        tree.children(parent)
            .any(|c| !tree.node(c).is_dir() && tree.extension(c).is_some_and(|e| exts.contains(&e.as_str())))
    };
    let lower_path = || tree.path(id).to_string_lossy().to_lowercase();
    let contents = Action::Contents { min_age: 0 };

    // Proyectos de desarrollo: todo se regenera con el gestor de paquetes o al
    // compilar. Nunca dentro de programas instalados.
    let project = if eq(name, "node_modules") && sibling(false, "package.json") {
        Some((C::DevDependencies, "node_modules"))
    } else if eq(name, "target") && sibling(false, "Cargo.toml") {
        Some((C::BuildOutputs, "Compilación de Rust (target)"))
    } else if ["__pycache__", ".pytest_cache", ".mypy_cache", ".ruff_cache"].iter().any(|n| eq(name, n)) {
        Some((C::BuildOutputs, "Caché de Python"))
    } else if (eq(name, "bin") || eq(name, "obj")) && sibling_ext(&["csproj", "vbproj", "fsproj"]) {
        Some((C::BuildOutputs, "Compilación de .NET (bin/obj)"))
    } else if eq(name, "build") && (sibling(false, "build.gradle") || sibling(false, "build.gradle.kts")) {
        Some((C::BuildOutputs, "Compilación de Gradle"))
    } else if [".next", ".nuxt", ".svelte-kit", ".parcel-cache", ".turbo", ".angular"].iter().any(|n| eq(name, n))
        && sibling(false, "package.json")
    {
        Some((C::BuildOutputs, "Caché de compilación web"))
    } else {
        None
    };
    if let Some((category, label)) = project {
        let path = lower_path();
        // Las cachés de Python de una instalación (Lib, site-packages) son parte del programa.
        let python_install =
            label == "Caché de Python" && (path.contains("\\lib\\") || path.contains("\\site-packages\\"));
        if in_app_place(&path, prefixes) || python_install {
            return None;
        }
        return Some((category, S::Regenerable, Action::Folder, label));
    }

    // Lanzadores de juegos.
    if eq(pname, "steamapps") {
        if eq(name, "shadercache") {
            return Some((C::GameCache, S::Regenerable, contents, "Caché de sombreadores de Steam"));
        }
        if eq(name, "downloading") {
            return Some((C::GameCache, S::Review, contents, "Descargas incompletas de Steam"));
        }
        if eq(name, "temp") {
            return Some((C::GameCache, S::Safe, contents, "Temporales de Steam"));
        }
    }
    if eq(pname, "appcache") && eq(name, "httpcache") {
        return Some((C::GameCache, S::Safe, contents, "Caché web de Steam"));
    }
    if (eq(name, "logs") || eq(name, "dumps")) && sibling(true, "steamapps") {
        return Some((C::GameCache, S::Safe, contents, "Registros y volcados de Steam"));
    }
    if name.len() >= 8
        && name[..8].eq_ignore_ascii_case(b"webcache")
        && lower_path().contains("\\epicgameslauncher\\saved\\")
    {
        return Some((C::GameCache, S::Safe, contents, "Caché web de Epic Games"));
    }

    // Navegadores y aplicaciones basadas en Chromium/Electron (siempre bajo AppData).
    let service_worker = (eq(name, "cachestorage") || eq(name, "scriptcache")) && eq(pname, "service worker");
    let chromium_cache = eq(name, "cache")
        && (sibling(true, "GPUCache")
            || sibling(true, "Code Cache")
            || sibling(false, "Preferences")
            || sibling(false, "Local State"));
    if CHROMIUM_CACHES.iter().any(|n| eq(name, n)) || service_worker || chromium_cache {
        let path = lower_path();
        if path.contains("\\appdata\\") {
            return Some(if BROWSERS.iter().any(|b| path.contains(b)) {
                (C::BrowserCache, S::Safe, contents, "Caché de navegador")
            } else {
                (C::AppCache, S::Safe, contents, "Caché de aplicación")
            });
        }
    }
    if eq(name, "cache2") && lower_path().contains("\\mozilla\\firefox\\profiles\\") {
        return Some((C::BrowserCache, S::Safe, contents, "Caché de Firefox"));
    }
    None
}

/// Discos virtuales grandes de Docker y WSL: no se borran, se compactan.
fn virtual_disk_advice(tree: &Tree, id: NodeId) -> Option<JunkItem> {
    let node = tree.node(id);
    if node.alloc < 4 * GB || tree.extension(id).as_deref() != Some("vhdx") {
        return None;
    }
    let path = tree.path(id).to_string_lossy().to_lowercase();
    let (text, label) = if path.contains("\\docker\\") {
        (ADVICE_DOCKER, "Disco virtual de Docker")
    } else if path.contains("\\packages\\") || path.contains("\\wsl\\") {
        (ADVICE_WSL, "Disco virtual de WSL")
    } else {
        return None;
    };
    Some(JunkItem {
        category: C::Advice,
        safety: S::Advice,
        action: Action::Advice(text),
        label,
        nodes: vec![id],
        bytes: node.alloc,
        files: 1,
        needs_admin: false,
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::clean::Location;
    use crate::tree::tests::tree_from;

    const MB: u64 = 1 << 20;

    fn labels(r: &JunkReport) -> Vec<(&'static str, u64)> {
        r.items.iter().map(|i| (i.label, i.bytes)).collect()
    }

    #[test]
    fn patrones_de_proyectos() {
        // R:\
        //   web\package.json, web\node_modules\a\b.js (anidado: solo cuenta el de arriba)
        //   rust\Cargo.toml, rust\target\x.exe
        //   suelto\node_modules\c.js (sin package.json: no es basura)
        let t = tree_from(
            &[
                (0, "web", true, 0),           // 1
                (0, "rust", true, 0),          // 2
                (0, "suelto", true, 0),        // 3
                (1, "package.json", false, 1), // 4
                (1, "node_modules", true, 0),  // 5
                (2, "Cargo.toml", false, 1),   // 6
                (2, "target", true, 0),        // 7
                (3, "node_modules", true, 0),  // 8
                (5, "a", true, 0),             // 9
                (7, "x.exe", false, 30 * MB),  // 10
                (8, "c.js", false, 5 * MB),    // 11
                (9, "b.js", false, 10 * MB),   // 12
                (9, "node_modules", true, 0),  // 13
                (9, "package.json", false, 0), // 14
                (13, "d.js", false, 2 * MB),   // 15
            ],
            None,
        );
        let r = find_junk(&t, &JunkContext::empty(1_800_000_000));
        assert_eq!(labels(&r), vec![("node_modules", 12 * MB), ("Compilación de Rust (target)", 30 * MB)]);
        assert_eq!(r.items[0].action, Action::Folder);
    }

    #[test]
    fn programas_instalados_no_son_proyectos() {
        // node_modules dentro de un programa (VS Code en AppData\Local\Programs): no se toca.
        let t = tree_from(
            &[
                (0, "AppData", true, 0),       // 1
                (1, "Programs", true, 0),      // 2
                (2, "app", true, 0),           // 3
                (3, "package.json", false, 0), // 4
                (3, "node_modules", true, 0),  // 5
                (5, "lib.js", false, 50 * MB), // 6
            ],
            None,
        );
        let r = find_junk(&t, &JunkContext::empty(1_800_000_000));
        assert!(r.items.is_empty(), "{:?}", labels(&r));
    }

    #[test]
    fn caches_de_chromium_y_steam() {
        let t = tree_from(
            &[
                (0, "AppData", true, 0),           // 1
                (0, "Steam", true, 0),             // 2
                (1, "Local", true, 0),             // 3
                (2, "steamapps", true, 0),         // 4
                (3, "BraveSoftware", true, 0),     // 5
                (3, "discord", true, 0),           // 6
                (4, "shadercache", true, 0),       // 7
                (4, "common", true, 0),            // 8
                (5, "Default", true, 0),           // 9
                (6, "Cache", true, 0),             // 10
                (6, "GPUCache", true, 0),          // 11
                (7, "s.bin", false, 40 * MB),      // 12
                (8, "juego.pak", false, 900 * MB), // 13
                (9, "Code Cache", true, 0),        // 14
                (10, "data_1", false, 20 * MB),    // 15
                (11, "g", false, MB),              // 16
                (14, "js", false, 7 * MB),         // 17
            ],
            None,
        );
        let r = find_junk(&t, &JunkContext::empty(1_800_000_000));
        let mut got = labels(&r);
        got.sort();
        assert_eq!(
            got,
            vec![
                ("Caché de aplicación", MB),
                ("Caché de aplicación", 20 * MB),
                ("Caché de navegador", 7 * MB),
                ("Caché de sombreadores de Steam", 40 * MB),
            ]
        );
        assert!(r.items.iter().all(|i| i.label != "juego.pak"), "los juegos no se tocan");
    }

    #[test]
    fn ubicaciones_fijas_y_antiguedad() {
        let now = 1_800_000_000;
        let t = tree_from(
            &[
                (0, "Temp", true, 0),            // 1
                (1, "viejo.tmp", false, 3 * MB), // 2
                (1, "nuevo.tmp", false, 5 * MB), // 3
            ],
            None,
        );
        // La antigüedad mínima se prueba con archivos reales en tests/limpieza.rs.
        let mut ctx = JunkContext::empty(now);
        ctx.locations.push(Location {
            path: PathBuf::from("R:\\Temp"),
            category: C::Temp,
            safety: S::Safe,
            action: Action::Contents { min_age: 0 },
            label: "Temporales del usuario",
            needs_admin: false,
        });
        let r = find_junk(&t, &ctx);
        assert_eq!(labels(&r), vec![("Temporales del usuario", 8 * MB)]);
        assert_eq!(r.items[0].files, 2);
        assert_eq!(r.total(S::Safe), 8 * MB);
    }
}
