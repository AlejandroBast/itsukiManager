//! Limpieza inteligente: detecta basura conocida en un árbol ya escaneado.
//!
//! Hay dos tipos de reglas:
//! - **Ubicaciones fijas** (`%TEMP%`, cachés de GPU, Windows Update…), que se
//!   resuelven con las variables de entorno.
//! - **Patrones** que se buscan en todo el árbol: `node_modules` junto a un
//!   `package.json`, `target` junto a un `Cargo.toml`, cachés de Chromium y
//!   Electron, cachés de Steam y Epic…
//!
//! Cada hallazgo lleva un nivel de [`Safety`] y la [`Action`] que lo limpia.
//! Detectar es instantáneo porque trabaja sobre el árbol en memoria; el
//! borrado está en [`delete`].

pub mod delete;
mod rules;

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::tree::{NodeId, Tree};

pub use delete::{DeleteReport, clean_item};
pub use rules::find_junk;

/// Qué tan seguro es borrar algo.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub enum Safety {
    /// Se regenera solo y sin coste apreciable (temporales, cachés).
    Safe,
    /// Se puede volver a generar o descargar, pero cuesta tiempo.
    Regenerable,
    /// Puede contener cosas tuyas: revísalo antes.
    Review,
    /// No se borra desde aquí: es una recomendación.
    Advice,
}

impl Safety {
    pub fn label(self) -> &'static str {
        match self {
            Safety::Safe => "Seguro",
            Safety::Regenerable => "Regenerable",
            Safety::Review => "Revisar",
            Safety::Advice => "Consejo",
        }
    }
}

/// Tipo de basura.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub enum JunkCategory {
    Temp,
    BrowserCache,
    AppCache,
    ShaderCache,
    WindowsUpdate,
    CrashDumps,
    DevDependencies,
    BuildOutputs,
    PackageCaches,
    GameCache,
    RecycleBin,
    OldDownloads,
    Advice,
}

impl JunkCategory {
    pub const ALL: [JunkCategory; 13] = [
        JunkCategory::Temp,
        JunkCategory::BrowserCache,
        JunkCategory::AppCache,
        JunkCategory::ShaderCache,
        JunkCategory::WindowsUpdate,
        JunkCategory::CrashDumps,
        JunkCategory::DevDependencies,
        JunkCategory::BuildOutputs,
        JunkCategory::PackageCaches,
        JunkCategory::GameCache,
        JunkCategory::RecycleBin,
        JunkCategory::OldDownloads,
        JunkCategory::Advice,
    ];

    pub fn label(self) -> &'static str {
        match self {
            JunkCategory::Temp => "Archivos temporales",
            JunkCategory::BrowserCache => "Caché de navegadores",
            JunkCategory::AppCache => "Caché de aplicaciones",
            JunkCategory::ShaderCache => "Caché de sombreadores (GPU)",
            JunkCategory::WindowsUpdate => "Descargas de Windows Update",
            JunkCategory::CrashDumps => "Volcados de errores e informes",
            JunkCategory::DevDependencies => "Dependencias de proyectos",
            JunkCategory::BuildOutputs => "Compilaciones de proyectos",
            JunkCategory::PackageCaches => "Cachés de gestores de paquetes",
            JunkCategory::GameCache => "Cachés de lanzadores de juegos",
            JunkCategory::RecycleBin => "Papelera de reciclaje",
            JunkCategory::OldDownloads => "Instaladores antiguos en Descargas",
            JunkCategory::Advice => "Sugerencias",
        }
    }

    /// Nombre corto para la línea de comandos.
    pub fn id(self) -> &'static str {
        match self {
            JunkCategory::Temp => "temporales",
            JunkCategory::BrowserCache => "navegadores",
            JunkCategory::AppCache => "aplicaciones",
            JunkCategory::ShaderCache => "sombreadores",
            JunkCategory::WindowsUpdate => "windows-update",
            JunkCategory::CrashDumps => "volcados",
            JunkCategory::DevDependencies => "dependencias",
            JunkCategory::BuildOutputs => "compilaciones",
            JunkCategory::PackageCaches => "paquetes",
            JunkCategory::GameCache => "juegos",
            JunkCategory::RecycleBin => "papelera",
            JunkCategory::OldDownloads => "descargas",
            JunkCategory::Advice => "sugerencias",
        }
    }

    pub fn from_id(id: &str) -> Option<JunkCategory> {
        JunkCategory::ALL.into_iter().find(|c| c.id().eq_ignore_ascii_case(id))
    }
}

/// Cómo se limpia un hallazgo.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum Action {
    /// Borrar el contenido de la carpeta (se conserva la carpeta). Solo las
    /// entradas modificadas hace más de `min_age` segundos.
    Contents { min_age: i64 },
    /// Borrar la carpeta entera.
    Folder,
    /// Borrar los archivos indicados.
    Files,
    /// Vaciar la Papelera de reciclaje de la unidad.
    EmptyRecycleBin,
    /// No se borra: recomendación para hacerlo a mano.
    Advice(&'static str),
}

/// Un hallazgo de basura.
#[derive(Clone, Debug, Serialize)]
pub struct JunkItem {
    pub category: JunkCategory,
    pub safety: Safety,
    pub action: Action,
    /// Descripción corta ("Caché DirectX de NVIDIA", "node_modules"…).
    pub label: &'static str,
    /// Carpeta (o archivos, en [`Action::Files`]) afectados.
    pub nodes: Vec<NodeId>,
    /// Espacio en disco que se liberaría.
    pub bytes: u64,
    pub files: u64,
    /// Hace falta ser administrador para limpiarlo.
    pub needs_admin: bool,
}

/// Resumen por categoría.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct CategoryTotal {
    pub category: JunkCategory,
    /// Nivel más permisivo de sus hallazgos (el que se muestra).
    pub safety: Safety,
    pub items: usize,
    pub bytes: u64,
    pub files: u64,
}

/// Resultado de buscar basura.
#[derive(Clone, Debug, Default, Serialize)]
pub struct JunkReport {
    pub items: Vec<JunkItem>,
}

impl JunkReport {
    /// Espacio liberable con un nivel de seguridad concreto.
    pub fn total(&self, safety: Safety) -> u64 {
        self.items.iter().filter(|i| i.safety == safety).map(|i| i.bytes).sum()
    }

    /// Totales por categoría, de mayor a menor.
    pub fn by_category(&self) -> Vec<CategoryTotal> {
        let mut out: Vec<CategoryTotal> = Vec::new();
        for item in &self.items {
            match out.iter_mut().find(|c| c.category == item.category) {
                Some(c) => {
                    c.items += 1;
                    c.bytes += item.bytes;
                    c.files += item.files;
                    c.safety = c.safety.min(item.safety);
                }
                None => out.push(CategoryTotal {
                    category: item.category,
                    safety: item.safety,
                    items: 1,
                    bytes: item.bytes,
                    files: item.files,
                }),
            }
        }
        out.sort_by(|a, b| {
            (a.category == JunkCategory::Advice).cmp(&(b.category == JunkCategory::Advice)).then(b.bytes.cmp(&a.bytes))
        });
        out
    }
}

/// Una ubicación fija de basura.
#[derive(Clone, Debug)]
pub struct Location {
    pub path: PathBuf,
    pub category: JunkCategory,
    pub safety: Safety,
    pub action: Action,
    pub label: &'static str,
    pub needs_admin: bool,
}

/// Contexto de la búsqueda: ubicaciones resueltas y umbrales.
#[derive(Clone, Debug)]
pub struct JunkContext {
    /// Momento actual en segundos Unix.
    pub now: i64,
    /// Antigüedad mínima de lo que se borra en carpetas temporales.
    pub temp_min_age: i64,
    /// Antigüedad mínima de los instaladores de Descargas.
    pub downloads_min_age: i64,
    /// Tamaño mínimo de los instaladores de Descargas.
    pub downloads_min_size: u64,
    /// Los hallazgos más pequeños que esto no se muestran (ruido).
    pub min_item: u64,
    pub locations: Vec<Location>,
    /// Carpeta de descargas del usuario.
    pub downloads: Option<PathBuf>,
    /// Carpeta de Windows (para las sugerencias sobre WinSxS).
    pub windows: Option<PathBuf>,
}

const DAY: i64 = 86_400;

impl JunkContext {
    /// Contexto del usuario actual a partir de las variables de entorno.
    pub fn from_env() -> JunkContext {
        let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
        let local = env("LOCALAPPDATA");
        let profile = env("USERPROFILE");
        let windir = env("SystemRoot");
        let pdata = env("ProgramData");
        let sysdrive = env("SystemDrive").map(|d| {
            let mut s = d.into_os_string();
            s.push("\\");
            PathBuf::from(s)
        });
        let temp_age = DAY;

        let mut locations = Vec::new();
        let mut add = |base: &Option<PathBuf>, rel: &str, category, safety, action: Action, label, needs_admin| {
            if let Some(b) = base {
                let path = if rel.is_empty() { b.clone() } else { b.join(rel) };
                locations.push(Location { path, category, safety, action, label, needs_admin });
            }
        };
        use JunkCategory as C;
        use Safety as S;
        let contents = Action::Contents { min_age: 0 };
        let temp = Some(std::env::temp_dir());
        add(&temp, "", C::Temp, S::Safe, Action::Contents { min_age: temp_age }, "Temporales del usuario", false);
        add(&windir, "Temp", C::Temp, S::Safe, Action::Contents { min_age: temp_age }, "Temporales de Windows", true);
        add(&sysdrive, "AMD", C::Temp, S::Safe, contents.clone(), "Instaladores extraídos de AMD", false);
        add(
            &pdata,
            "NVIDIA Corporation\\Downloader",
            C::Temp,
            S::Safe,
            contents.clone(),
            "Descargas del instalador de NVIDIA",
            true,
        );
        add(&local, "D3DSCache", C::ShaderCache, S::Safe, contents.clone(), "Caché de sombreadores de DirectX", false);
        add(&local, "NVIDIA\\DXCache", C::ShaderCache, S::Safe, contents.clone(), "Caché DirectX de NVIDIA", false);
        add(&local, "NVIDIA\\GLCache", C::ShaderCache, S::Safe, contents.clone(), "Caché OpenGL de NVIDIA", false);
        add(&pdata, "NVIDIA Corporation\\NV_Cache", C::ShaderCache, S::Safe, contents.clone(), "Caché de NVIDIA", true);
        add(&local, "AMD\\DxCache", C::ShaderCache, S::Safe, contents.clone(), "Caché DirectX de AMD", false);
        add(&local, "AMD\\DxcCache", C::ShaderCache, S::Safe, contents.clone(), "Caché DXC de AMD", false);
        add(&local, "AMD\\GLCache", C::ShaderCache, S::Safe, contents.clone(), "Caché OpenGL de AMD", false);
        add(&local, "AMD\\VkCache", C::ShaderCache, S::Safe, contents.clone(), "Caché Vulkan de AMD", false);
        add(
            &local,
            "Intel\\ShaderCache",
            C::ShaderCache,
            S::Safe,
            contents.clone(),
            "Caché de sombreadores de Intel",
            false,
        );
        add(
            &windir,
            "SoftwareDistribution\\Download",
            C::WindowsUpdate,
            S::Safe,
            contents.clone(),
            "Descargas de Windows Update",
            true,
        );
        add(
            &windir,
            "ServiceProfiles\\NetworkService\\AppData\\Local\\Microsoft\\Windows\\DeliveryOptimization\\Cache",
            C::WindowsUpdate,
            S::Safe,
            contents.clone(),
            "Caché de optimización de distribución",
            true,
        );
        add(&local, "CrashDumps", C::CrashDumps, S::Safe, contents.clone(), "Volcados de aplicaciones", false);
        add(
            &local,
            "Microsoft\\Windows\\WER\\ReportArchive",
            C::CrashDumps,
            S::Safe,
            contents.clone(),
            "Informes de errores",
            false,
        );
        add(
            &local,
            "Microsoft\\Windows\\WER\\ReportQueue",
            C::CrashDumps,
            S::Safe,
            contents.clone(),
            "Informes de errores",
            false,
        );
        add(
            &pdata,
            "Microsoft\\Windows\\WER\\ReportArchive",
            C::CrashDumps,
            S::Safe,
            contents.clone(),
            "Informes de errores del sistema",
            true,
        );
        add(
            &pdata,
            "Microsoft\\Windows\\WER\\ReportQueue",
            C::CrashDumps,
            S::Safe,
            contents.clone(),
            "Informes de errores del sistema",
            true,
        );
        add(&windir, "Minidump", C::CrashDumps, S::Safe, contents.clone(), "Minivolcados de Windows", true);
        add(&windir, "LiveKernelReports", C::CrashDumps, S::Safe, contents.clone(), "Informes del kernel", true);
        add(&windir, "MEMORY.DMP", C::CrashDumps, S::Safe, Action::Files, "Volcado de memoria de Windows", true);
        add(
            &local,
            "Microsoft\\Windows\\INetCache",
            C::BrowserCache,
            S::Safe,
            contents.clone(),
            "Caché de Internet de Windows",
            false,
        );
        add(&local, "npm-cache", C::PackageCaches, S::Regenerable, contents.clone(), "Caché de npm", false);
        add(&local, "pip\\cache", C::PackageCaches, S::Regenerable, contents.clone(), "Caché de pip", false);
        add(&local, "Yarn\\Cache", C::PackageCaches, S::Regenerable, contents.clone(), "Caché de Yarn", false);
        add(&local, "NuGet\\v3-cache", C::PackageCaches, S::Regenerable, contents.clone(), "Caché de NuGet", false);
        add(&local, "go-build", C::PackageCaches, S::Regenerable, contents.clone(), "Caché de Go", false);
        add(
            &profile,
            ".cargo\\registry\\cache",
            C::PackageCaches,
            S::Regenerable,
            contents.clone(),
            "Caché de crates de Cargo",
            false,
        );
        add(&profile, ".gradle\\caches", C::PackageCaches, S::Regenerable, contents, "Caché de Gradle", false);

        JunkContext {
            now: crate::format::now_unix(),
            temp_min_age: temp_age,
            downloads_min_age: 30 * DAY,
            downloads_min_size: 50 << 20,
            min_item: 64 << 10,
            locations,
            downloads: profile.map(|p| p.join("Downloads")),
            windows: windir,
        }
    }

    /// Contexto vacío (sin ubicaciones fijas), útil para pruebas.
    pub fn empty(now: i64) -> JunkContext {
        JunkContext {
            now,
            temp_min_age: DAY,
            downloads_min_age: 30 * DAY,
            downloads_min_size: 50 << 20,
            min_item: 0,
            locations: Vec::new(),
            downloads: None,
            windows: None,
        }
    }
}

/// Ruta para mostrar de un hallazgo (la carpeta, o el primer archivo).
pub fn item_path(tree: &Tree, item: &JunkItem) -> Option<PathBuf> {
    item.nodes.first().map(|&id| tree.path(id))
}

/// `true` si `path` está dentro de `base` (comparando sin mayúsculas).
pub(crate) fn path_starts_with(path: &Path, base: &Path) -> bool {
    let p = path.to_string_lossy().to_lowercase();
    let b = base.to_string_lossy().to_lowercase();
    let b = b.trim_end_matches('\\');
    p == b || p.starts_with(&format!("{b}\\"))
}
