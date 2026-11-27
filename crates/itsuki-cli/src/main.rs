//! `itsuki`: línea de comandos de itsukiManager.

mod ui;

use std::borrow::Cow;
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};

use clap::{Args, Parser, Subcommand, ValueEnum};
use itsuki_core::format::{self, count, percent, size};
use itsuki_core::{
    DirMode, DirQuery, Engine, FileQuery, Metric, NodeId, ROOT, ScanOptions, ScanProgress, Tree, actions, flags,
    system, type_report,
};
use serde_json::json;

use ui::{Cell, Style, Table, bar};

#[derive(Parser)]
#[command(
    name = "itsuki",
    version,
    about = "itsukiManager · análisis ultrarrápido del almacenamiento",
    after_help = "Ejemplos:\n  itsuki carpetas                 Carpetas de 1 GB o más en C:\\\n  itsuki carpetas D:\\ --modo hojas --min 5GB\n  itsuki archivos --top 50 --antiguos 365\n  itsuki arbol C:\\Users -p 2\n  itsuki informe"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
    #[command(flatten)]
    global: Global,
}

#[derive(Args)]
struct Global {
    /// Hilos de escaneo (0 = automático)
    #[arg(long, global = true, default_value_t = 0, value_name = "N")]
    hilos: usize,
    /// Tamaño a usar: disco (espacio real ocupado) o datos (tamaño de los archivos)
    #[arg(long, global = true, value_enum, default_value_t = MetricArg::Disco)]
    metrica: MetricArg,
    /// Contar varias veces los archivos con enlaces duros
    #[arg(long, global = true)]
    contar_enlaces_duros: bool,
    /// Motor de escaneo
    #[arg(long, global = true, value_enum, default_value_t = EngineArg::Auto)]
    motor: EngineArg,
    /// No mostrar el progreso del escaneo
    #[arg(long, short = 'q', global = true)]
    silencioso: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum MetricArg {
    Disco,
    Datos,
}

#[derive(Clone, Copy, ValueEnum)]
enum EngineArg {
    /// MFT si es una unidad NTFS completa y eres administrador; si no, nativo
    Auto,
    /// Lee la tabla maestra de NTFS (el más rápido; requiere administrador)
    Mft,
    /// API nativa de Windows
    Nativo,
    /// Biblioteca estándar (lento; para comparar)
    Portable,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ModoArg {
    /// Cada carpeta que supera el mínimo
    Todas,
    /// Solo las carpetas que de verdad ocupan el espacio
    Hojas,
}

#[derive(Subcommand)]
enum Command {
    /// Lista las carpetas que superan un tamaño (por defecto 1 GB)
    Carpetas(CarpetasArgs),
    /// Lista los archivos más grandes
    Archivos(ArchivosArgs),
    /// Muestra el espacio por tipo de archivo
    Tipos(TiposArgs),
    /// Dibuja el árbol de carpetas con sus tamaños
    Arbol(ArbolArgs),
    /// Resumen completo: carpetas, archivos y tipos
    Informe(RutaArgs),
    /// Envía archivos o carpetas a la Papelera (pide confirmación)
    Papelera(PapeleraArgs),
    /// Muestra un archivo o carpeta en el Explorador
    Abrir { ruta: PathBuf },
    /// Muestra las unidades y los permisos
    Info,
    /// Compara los motores de escaneo sobre la misma ruta (diagnóstico)
    #[command(hide = true)]
    Comparar(RutaArgs),
}

#[derive(Args)]
struct RutaArgs {
    /// Carpeta o unidad a analizar (por defecto la unidad del sistema)
    ruta: Option<PathBuf>,
}

#[derive(Args)]
struct ExportArgs {
    /// Guarda el resultado en CSV (separado por ';', se abre en Excel)
    #[arg(long, value_name = "ARCHIVO")]
    csv: Option<PathBuf>,
    /// Guarda el resultado en JSON
    #[arg(long, value_name = "ARCHIVO")]
    json: Option<PathBuf>,
}

#[derive(Args)]
struct CarpetasArgs {
    #[command(flatten)]
    ruta: RutaArgs,
    /// Tamaño mínimo (ej.: 500MB, 1GB, 2,5GB)
    #[arg(long, default_value = "1GB", value_parser = parse_size_arg)]
    min: u64,
    /// Profundidad máxima (por defecto 4 en modo "todas" y sin límite en "hojas")
    #[arg(long, short = 'p')]
    prof: Option<u32>,
    /// Qué carpetas listar
    #[arg(long, value_enum, default_value_t = ModoArg::Todas)]
    modo: ModoArg,
    /// Mostrar solo las N primeras
    #[arg(long)]
    top: Option<usize>,
    #[command(flatten)]
    export: ExportArgs,
}

#[derive(Args)]
struct ArchivosArgs {
    #[command(flatten)]
    ruta: RutaArgs,
    /// Cuántos archivos mostrar
    #[arg(long, default_value_t = 30)]
    top: usize,
    /// Tamaño mínimo (ej.: 100MB)
    #[arg(long, default_value = "0", value_parser = parse_size_arg)]
    min: u64,
    /// Solo archivos sin modificar desde hace al menos N días
    #[arg(long, value_name = "DIAS")]
    antiguos: Option<u32>,
    #[command(flatten)]
    export: ExportArgs,
}

#[derive(Args)]
struct TiposArgs {
    #[command(flatten)]
    ruta: RutaArgs,
    /// Cuántas extensiones mostrar
    #[arg(long, default_value_t = 15)]
    top: usize,
}

#[derive(Args)]
struct ArbolArgs {
    #[command(flatten)]
    ruta: RutaArgs,
    /// Niveles a mostrar
    #[arg(long, short = 'p', default_value_t = 3)]
    prof: u32,
    /// Ocultar carpetas menores que este tamaño
    #[arg(long, default_value = "1GB", value_parser = parse_size_arg)]
    min: u64,
}

#[derive(Args)]
struct PapeleraArgs {
    /// Archivos o carpetas a enviar a la Papelera
    #[arg(required = true)]
    rutas: Vec<PathBuf>,
    /// No pedir confirmación
    #[arg(long)]
    si: bool,
}

fn parse_size_arg(s: &str) -> Result<u64, String> {
    format::parse_size(s).ok_or_else(|| format!("tamaño no válido: {s} (ejemplos: 500MB, 1GB, 2,5GB)"))
}

type Res = Result<(), Box<dyn std::error::Error>>;

fn main() -> ExitCode {
    let cli = Cli::parse();
    let style = Style::detect();
    let ctx = Ctx { style, global: &cli.global };
    let result = match &cli.command {
        Command::Carpetas(a) => ctx.carpetas(a),
        Command::Archivos(a) => ctx.archivos(a),
        Command::Tipos(a) => ctx.tipos(a),
        Command::Arbol(a) => ctx.arbol(a),
        Command::Informe(a) => ctx.informe(a),
        Command::Papelera(a) => ctx.papelera(a),
        Command::Abrir { ruta } => actions::reveal_in_explorer(ruta).map_err(Into::into),
        Command::Info => ctx.info(),
        Command::Comparar(a) => ctx.comparar(a),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{} {e}", style.red("error:"));
            ExitCode::FAILURE
        }
    }
}

struct Ctx<'a> {
    style: Style,
    global: &'a Global,
}

impl Ctx<'_> {
    fn metric(&self) -> Metric {
        match self.global.metrica {
            MetricArg::Disco => Metric::Disk,
            MetricArg::Datos => Metric::Logical,
        }
    }

    fn metric_label(&self) -> &'static str {
        match self.metric() {
            Metric::Disk => "En disco",
            Metric::Logical => "Tamaño",
        }
    }

    /// Escanea mostrando el progreso.
    fn scan(&self, ruta: &RutaArgs, quiet: bool) -> Result<Tree, Box<dyn std::error::Error>> {
        let engine = match self.global.motor {
            EngineArg::Auto => Engine::Auto,
            EngineArg::Mft => Engine::Mft,
            EngineArg::Nativo => Engine::Native,
            EngineArg::Portable => Engine::Portable,
        };
        self.scan_with(ruta, quiet, engine)
    }

    fn scan_with(&self, ruta: &RutaArgs, quiet: bool, engine: Engine) -> Result<Tree, Box<dyn std::error::Error>> {
        let root = ruta.ruta.clone().unwrap_or_else(default_root);
        let opts =
            ScanOptions { threads: self.global.hilos, dedupe_hardlinks: !self.global.contar_enlaces_duros, engine };
        let progress = ScanProgress::new();
        let show = !quiet && !self.global.silencioso && io::IsTerminal::is_terminal(&io::stderr());
        let done = AtomicBool::new(false);
        let tree = std::thread::scope(|s| {
            if show {
                s.spawn(|| ui::progress_loop(&progress, &done));
            }
            let r = itsuki_core::scan(&root, &opts, &progress);
            done.store(true, Ordering::Relaxed);
            r
        })?;
        Ok(tree)
    }

    fn header(&self, tree: &Tree) {
        let st = self.style;
        let root = tree.node(ROOT);
        let s = tree.stats();
        println!("{} {}", st.title("itsukiManager"), st.bold(&tree.root_path().display().to_string()));
        println!(
            "  {} en disco · {} de datos · {} archivos · {} carpetas · {} · motor {} · {} hilos",
            st.bold(&size(root.alloc)),
            size(root.size),
            count(s.files),
            count(s.dirs),
            format::seconds(s.elapsed),
            s.engine,
            s.threads
        );
        if let Some(e) = &s.mft_error {
            println!("  {}", st.dim(&format!("No se pudo usar el motor MFT ({e}); se usó el nativo.")));
        }
        if let Some(vol) = system::volume_of(tree.root_path()) {
            if same_path(&vol.root, tree.root_path()) {
                let missing = vol.used().saturating_sub(root.alloc);
                println!(
                    "  Unidad {}: {} · usado {} · libre {} · sin contabilizar {}",
                    vol.root.display(),
                    size(vol.total),
                    size(vol.used()),
                    st.green(&size(vol.free)),
                    st.dim(&size(missing))
                );
            }
        }
        if s.hardlinks > 0 {
            println!(
                "  {}",
                st.dim(&format!(
                    "{} archivos con varias rutas (enlaces duros) contados una sola vez",
                    count(s.hardlinks)
                ))
            );
        }
        if s.denied > 0 {
            let hint = if s.elevated {
                ""
            } else {
                " (como administrador se leen todas y se usa el motor MFT, mucho más rápido)"
            };
            println!("  {}", st.yellow(&format!("⚠ {} carpetas sin acceso{hint}", count(s.denied))));
        }
        println!();
    }

    fn carpetas(&self, a: &CarpetasArgs) -> Res {
        let tree = self.scan(&a.ruta, false)?;
        self.header(&tree);
        let metric = self.metric();
        let mode = if a.modo == ModoArg::Hojas { DirMode::Leaves } else { DirMode::All };
        let max_depth = a.prof.or(if a.modo == ModoArg::Todas { Some(4) } else { None });
        let hits = tree.large_dirs(&DirQuery { min: a.min, max_depth, mode, metric });
        if hits.is_empty() {
            println!("  No hay carpetas de {} o más.", size(a.min));
            return Ok(());
        }
        let shown = a.top.map_or(hits.len(), |t| t.min(hits.len()));
        let total = metric.of(tree.node(ROOT));
        let mut table = Table::new(&[
            (self.metric_label(), true),
            ("%", true),
            ("", false),
            ("Archivos", true),
            ("Nivel", true),
            ("Carpeta", false),
        ]);
        for h in &hits[..shown] {
            let n = tree.node(h.id);
            let v = metric.of(n);
            table.row(vec![
                Cell::styled(size(v), Style::bold),
                Cell::plain(percent(v, total)),
                Cell::styled(bar(ratio(v, total), 10), Style::cyan),
                Cell::plain(count(u64::from(n.files))),
                Cell::plain(h.depth.to_string()),
                Cell::plain(tree.path(h.id).display().to_string()),
            ]);
        }
        table.print(self.style);
        let modo = if mode == DirMode::Leaves { "hojas" } else { "todas" };
        let mut footer = format!("{} carpetas de {} o más (modo {modo}", count(hits.len() as u64), size(a.min));
        if let Some(p) = max_depth {
            footer.push_str(&format!(", hasta {p} niveles"));
        }
        footer.push(')');
        if shown < hits.len() {
            footer.push_str(&format!(" · mostrando {shown}"));
        }
        println!("\n  {}", self.style.dim(&footer));

        let rows = || {
            hits.iter().map(|h| {
                let n = tree.node(h.id);
                (tree.path(h.id), n, h.depth)
            })
        };
        if let Some(p) = &a.export.csv {
            write_csv(
                p,
                &["ruta", "en_disco_bytes", "tamano_bytes", "archivos", "nivel"],
                rows().map(|(path, n, d)| {
                    vec![
                        path.display().to_string(),
                        n.alloc.to_string(),
                        n.size.to_string(),
                        n.files.to_string(),
                        d.to_string(),
                    ]
                }),
            )?;
            println!("  {} {}", self.style.green("CSV guardado en"), p.display());
        }
        if let Some(p) = &a.export.json {
            let data: Vec<_> = rows()
                .map(|(path, n, d)| {
                    json!({ "ruta": path, "en_disco": n.alloc, "tamano": n.size, "archivos": n.files, "nivel": d })
                })
                .collect();
            write_json(p, &data)?;
            println!("  {} {}", self.style.green("JSON guardado en"), p.display());
        }
        Ok(())
    }

    fn archivos(&self, a: &ArchivosArgs) -> Res {
        let tree = self.scan(&a.ruta, false)?;
        self.header(&tree);
        let metric = self.metric();
        let modified_before = a.antiguos.map(|d| format::now_unix() - i64::from(d) * 86_400);
        let ids = tree.top_files(&FileQuery { top: a.top, min: a.min, modified_before, metric });
        if ids.is_empty() {
            println!("  No hay archivos que cumplan el filtro.");
            return Ok(());
        }
        let total = metric.of(tree.node(ROOT));
        let mut table = Table::new(&[
            (self.metric_label(), true),
            ("%", true),
            ("Modificado", false),
            ("Notas", false),
            ("Archivo", false),
        ]);
        for &id in &ids {
            let n = tree.node(id);
            let v = metric.of(n);
            table.row(vec![
                Cell::styled(size(v), Style::bold),
                Cell::plain(percent(v, total)),
                Cell::plain(format::date(n.mtime)),
                Cell::styled(notes(n), Style::dim),
                Cell::plain(tree.path(id).display().to_string()),
            ]);
        }
        table.print(self.style);
        let sum: u64 = ids.iter().map(|&id| metric.of(tree.node(id))).sum();
        println!("\n  {}", self.style.dim(&format!("{} archivos · {} en total", count(ids.len() as u64), size(sum))));

        if let Some(p) = &a.export.csv {
            write_csv(
                p,
                &["ruta", "en_disco_bytes", "tamano_bytes", "modificado"],
                ids.iter().map(|&id| {
                    let n = tree.node(id);
                    vec![
                        tree.path(id).display().to_string(),
                        n.alloc.to_string(),
                        n.size.to_string(),
                        format::date(n.mtime),
                    ]
                }),
            )?;
            println!("  {} {}", self.style.green("CSV guardado en"), p.display());
        }
        if let Some(p) = &a.export.json {
            let data: Vec<_> = ids
                .iter()
                .map(|&id| {
                    let n = tree.node(id);
                    json!({
                        "ruta": tree.path(id),
                        "en_disco": n.alloc,
                        "tamano": n.size,
                        "modificado": format::date(n.mtime),
                        "nube": n.has(flags::CLOUD),
                    })
                })
                .collect();
            write_json(p, &data)?;
            println!("  {} {}", self.style.green("JSON guardado en"), p.display());
        }
        Ok(())
    }

    fn tipos(&self, a: &TiposArgs) -> Res {
        let tree = self.scan(&a.ruta, false)?;
        self.header(&tree);
        self.print_types(&tree, usize::MAX, a.top);
        Ok(())
    }

    fn print_types(&self, tree: &Tree, max_categories: usize, max_ext: usize) {
        let report = type_report(tree);
        let metric = self.metric();
        let value = |s: &itsuki_core::TypeStat| if metric == Metric::Disk { s.alloc } else { s.size };
        let total: u64 = report.categories.iter().map(|c| value(&c.1)).sum();

        let mut cats = Table::new(&[
            (self.metric_label(), true),
            ("%", true),
            ("", false),
            ("Archivos", true),
            ("Categoría", false),
        ]);
        for (cat, stat) in report.categories.iter().take(max_categories) {
            let v = value(stat);
            cats.row(vec![
                Cell::styled(size(v), Style::bold),
                Cell::plain(percent(v, total)),
                Cell::styled(bar(ratio(v, total), 10), Style::cyan),
                Cell::plain(count(stat.files)),
                Cell::plain(cat.label()),
            ]);
        }
        cats.print(self.style);
        if max_ext == 0 {
            return;
        }
        println!();
        let mut exts =
            Table::new(&[(self.metric_label(), true), ("%", true), ("Archivos", true), ("Extensión", false)]);
        for (ext, stat) in report.extensions.iter().take(max_ext) {
            let v = value(stat);
            let name: Cow<str> = if ext.is_empty() { "(sin extensión)".into() } else { format!(".{ext}").into() };
            exts.row(vec![
                Cell::styled(size(v), Style::bold),
                Cell::plain(percent(v, total)),
                Cell::plain(count(stat.files)),
                Cell::plain(name),
            ]);
        }
        exts.print(self.style);
    }

    fn arbol(&self, a: &ArbolArgs) -> Res {
        let tree = self.scan(&a.ruta, false)?;
        self.header(&tree);
        let metric = self.metric();
        let total = metric.of(tree.node(ROOT));
        println!("  {}  {}", self.style.bold(&tree.root_path().display().to_string()), self.style.bold(&size(total)));
        self.tree_level(&tree, ROOT, "  ", 0, a, metric, total);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn tree_level(&self, tree: &Tree, id: NodeId, prefix: &str, depth: u32, a: &ArbolArgs, metric: Metric, total: u64) {
        if depth >= a.prof {
            return;
        }
        let kids: Vec<NodeId> = tree
            .children_sorted(id, metric)
            .into_iter()
            .filter(|&c| tree.node(c).is_dir() && metric.of(tree.node(c)) >= a.min)
            .collect();
        let loose = tree.direct_files_size(id, metric);
        let show_loose = loose >= a.min && !kids.is_empty();
        let count_lines = kids.len() + usize::from(show_loose);
        for (i, &c) in kids.iter().enumerate() {
            let last = i + 1 == count_lines;
            let v = metric.of(tree.node(c));
            println!(
                "{prefix}{}{}  {}  {}",
                self.style.dim(if last { "└── " } else { "├── " }),
                tree.name_lossy(c),
                self.style.bold(&size(v)),
                self.style.dim(&percent(v, total))
            );
            let next = format!("{prefix}{}", if last { "    " } else { "│   " });
            let next = if self.style.dim("x") == "x" { next } else { next.replace('│', &self.style.dim("│")) };
            self.tree_level(tree, c, &next, depth + 1, a, metric, total);
        }
        if show_loose {
            println!(
                "{prefix}{}{}  {}",
                self.style.dim("└── "),
                self.style.dim("[archivos sueltos]"),
                self.style.bold(&size(loose))
            );
        }
    }

    fn informe(&self, a: &RutaArgs) -> Res {
        let tree = self.scan(a, false)?;
        self.header(&tree);
        let st = self.style;
        let metric = self.metric();
        let total = metric.of(tree.node(ROOT));

        println!("  {}", st.title("Carpetas que más ocupan (1 GB o más, solo las que contienen el espacio)"));
        let hits = tree.large_dirs(&DirQuery { min: 1 << 30, max_depth: None, mode: DirMode::Leaves, metric });
        let mut t = Table::new(&[(self.metric_label(), true), ("%", true), ("", false), ("Carpeta", false)]);
        for h in hits.iter().take(15) {
            let v = metric.of(tree.node(h.id));
            t.row(vec![
                Cell::styled(size(v), Style::bold),
                Cell::plain(percent(v, total)),
                Cell::styled(bar(ratio(v, total), 10), Style::cyan),
                Cell::plain(tree.path(h.id).display().to_string()),
            ]);
        }
        if hits.is_empty() {
            println!("  Ninguna carpeta llega a 1 GB.");
        } else {
            t.print(st);
        }

        println!("\n  {}", st.title("Archivos más grandes"));
        let ids = tree.top_files(&FileQuery { top: 10, min: 0, modified_before: None, metric });
        let mut t = Table::new(&[(self.metric_label(), true), ("Modificado", false), ("Archivo", false)]);
        for &id in &ids {
            let n = tree.node(id);
            t.row(vec![
                Cell::styled(size(metric.of(n)), Style::bold),
                Cell::plain(format::date(n.mtime)),
                Cell::plain(tree.path(id).display().to_string()),
            ]);
        }
        t.print(st);

        println!("\n  {}", st.title("Espacio por tipo"));
        self.print_types(&tree, 8, 0);
        Ok(())
    }

    fn papelera(&self, a: &PapeleraArgs) -> Res {
        let st = self.style;
        let mut total = 0u64;
        let mut table = Table::new(&[("Tamaño", true), ("Elemento", false)]);
        for p in &a.rutas {
            if let Some(reason) = actions::protection_reason(p) {
                return Err(format!("{} está protegida: {reason}", p.display()).into());
            }
            let meta = std::fs::symlink_metadata(p).map_err(|e| format!("{}: {e}", p.display()))?;
            let bytes = if meta.is_dir() {
                let tree = self.scan(&RutaArgs { ruta: Some(p.clone()) }, true)?;
                tree.node(ROOT).alloc
            } else {
                meta.len()
            };
            total += bytes;
            table.row(vec![Cell::styled(size(bytes), Style::bold), Cell::plain(p.display().to_string())]);
        }
        table.print(st);
        if !a.si {
            print!(
                "\n  ¿Enviar {} elemento(s) ({}) a la Papelera? [s/N] ",
                count(a.rutas.len() as u64),
                st.bold(&size(total))
            );
            io::stdout().flush()?;
            let mut answer = String::new();
            io::stdin().read_line(&mut answer)?;
            if !matches!(answer.trim().to_lowercase().as_str(), "s" | "si" | "sí" | "y" | "yes") {
                println!("  Cancelado: no se movió nada.");
                return Ok(());
            }
        }
        actions::send_to_recycle_bin(&a.rutas)?;
        println!(
            "  {} {} se liberarán al vaciar la Papelera.",
            st.green("✓ Enviado a la Papelera."),
            st.bold(&size(total))
        );
        Ok(())
    }

    fn comparar(&self, a: &RutaArgs) -> Res {
        let st = self.style;
        let mut engines = Vec::new();
        if system::is_elevated() {
            engines.push(("mft", Engine::Mft));
        }
        engines.push(("nativo", Engine::Native));
        engines.push(("portable", Engine::Portable));
        let mut trees = Vec::new();
        for (name, engine) in engines {
            match self.scan_with(a, false, engine) {
                Ok(t) => trees.push((name, t)),
                Err(e) => println!("  {} motor {name}: {e}", st.yellow("⚠")),
            }
        }
        let Some((ref_name, reference)) = trees.first() else {
            return Err("ningún motor pudo escanear la ruta".into());
        };
        println!("{} {}\n", st.title("Comparación de motores"), st.bold(&reference.root_path().display().to_string()));
        let mut t = Table::new(&[
            ("Motor", false),
            ("Tiempo", true),
            ("En disco", true),
            ("Datos", true),
            ("Archivos", true),
            ("Carpetas", true),
            ("Sin acceso", true),
            ("Enlaces duros", true),
        ]);
        for (name, tree) in &trees {
            let r = tree.node(ROOT);
            let s = tree.stats();
            t.row(vec![
                Cell::styled(*name, Style::bold),
                Cell::plain(format::seconds(s.elapsed)),
                Cell::plain(size(r.alloc)),
                Cell::plain(size(r.size)),
                Cell::plain(count(s.files)),
                Cell::plain(count(s.dirs)),
                Cell::plain(count(s.denied)),
                Cell::plain(count(s.hardlinks)),
            ]);
        }
        t.print(st);

        for (name, other) in trees.iter().skip(1) {
            let index: HashMap<String, NodeId> =
                other.children(ROOT).map(|c| (other.name_lossy(c).to_lowercase(), c)).collect();
            let mut seen = Vec::new();
            let mut rows: Vec<(i128, String, u64, u64)> = Vec::new();
            for c in reference.children(ROOT) {
                let key = reference.name_lossy(c).to_lowercase();
                let x = reference.node(c).alloc;
                let y = index.get(&key).map_or(0, |&o| other.node(o).alloc);
                seen.push(key);
                rows.push((i128::from(x) - i128::from(y), reference.name_lossy(c).into_owned(), x, y));
            }
            for c in other.children(ROOT) {
                if !seen.contains(&other.name_lossy(c).to_lowercase()) {
                    let y = other.node(c).alloc;
                    rows.push((-i128::from(y), other.name_lossy(c).into_owned(), 0, y));
                }
            }
            rows.retain(|r| r.0.unsigned_abs() >= 1 << 20);
            rows.sort_by_key(|r| std::cmp::Reverse(r.0.unsigned_abs()));
            println!(
                "\n  {}",
                st.title(&format!("Diferencias {ref_name} − {name} por carpeta de primer nivel (≥ 1 MB)"))
            );
            if rows.is_empty() {
                println!("  Ninguna: los dos motores coinciden.");
                continue;
            }
            let mut d = Table::new(&[(ref_name, true), (name, true), ("Diferencia", true), ("Elemento", false)]);
            for (diff, label, x, y) in rows.into_iter().take(20) {
                d.row(vec![
                    Cell::plain(size(x)),
                    Cell::plain(size(y)),
                    Cell::styled(signed_size(diff), Style::bold),
                    Cell::plain(label),
                ]);
            }
            d.print(st);
        }
        Ok(())
    }

    fn info(&self) -> Res {
        let st = self.style;
        println!("{}", st.title("itsukiManager"));
        let elevated = system::is_elevated();
        println!(
            "  Permisos: {}",
            if elevated {
                st.green("administrador (se leen también las carpetas protegidas)")
            } else {
                st.yellow("usuario normal (algunas carpetas del sistema no se pueden leer)")
            }
        );
        println!("  Hilos de escaneo por defecto: {}", itsuki_core::scan::default_threads());
        println!(
            "  Motor MFT: {}\n",
            if elevated {
                st.green("disponible (escaneo de unidades NTFS completas en segundos)")
            } else {
                st.yellow("necesita administrador")
            }
        );
        let mut t = Table::new(&[
            ("Unidad", false),
            ("Tipo", false),
            ("Formato", false),
            ("Total", true),
            ("Usado", true),
            ("Libre", true),
            ("", false),
            ("Nombre", false),
        ]);
        for v in system::volumes() {
            let kind = match (v.kind, v.seek_penalty) {
                (system::DriveKind::Fixed, Some(true)) => "HDD".to_string(),
                (system::DriveKind::Fixed, Some(false)) => "SSD".to_string(),
                (k, _) => k.label().to_string(),
            };
            t.row(vec![
                Cell::styled(v.root.display().to_string(), Style::bold),
                Cell::plain(kind),
                Cell::plain(v.filesystem.clone()),
                Cell::plain(size(v.total)),
                Cell::plain(size(v.used())),
                Cell::styled(size(v.free), Style::green),
                Cell::styled(
                    format!("{} {}", bar(ratio(v.used(), v.total), 10), percent(v.used(), v.total)),
                    Style::cyan,
                ),
                Cell::plain(v.label.clone()),
            ]);
        }
        t.print(st);
        Ok(())
    }
}

/// Unidad del sistema (normalmente `C:\`).
fn default_root() -> PathBuf {
    if cfg!(windows) {
        let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        PathBuf::from(format!("{drive}\\"))
    } else {
        PathBuf::from("/")
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().trim_end_matches(['\\', '/']).to_lowercase();
    norm(a) == norm(b)
}

/// Diferencia con signo: `+1,50 GB` o `-200 MB`.
fn signed_size(d: i128) -> String {
    let abs = size(d.unsigned_abs().min(u128::from(u64::MAX)) as u64);
    if d < 0 { format!("-{abs}") } else { format!("+{abs}") }
}
fn ratio(part: u64, total: u64) -> f64 {
    if total == 0 { 0.0 } else { part as f64 / total as f64 }
}

fn notes(n: &itsuki_core::Node) -> String {
    let mut v = Vec::new();
    if n.has(flags::CLOUD) {
        v.push("nube");
    }
    if n.has(flags::COMPRESSED) {
        v.push("comprimido");
    }
    if n.has(flags::SPARSE) {
        v.push("disperso");
    }
    if n.has(flags::SYSTEM) {
        v.push("sistema");
    } else if n.has(flags::HIDDEN) {
        v.push("oculto");
    }
    v.join(", ")
}

/// CSV con BOM y `;` como separador (lo que espera Excel en español).
fn write_csv(path: &Path, headers: &[&str], rows: impl Iterator<Item = Vec<String>>) -> io::Result<()> {
    let mut w = BufWriter::new(File::create(path)?);
    w.write_all("\u{FEFF}".as_bytes())?;
    writeln!(w, "{}", headers.join(";"))?;
    for row in rows {
        let cells: Vec<Cow<str>> = row
            .iter()
            .map(|c| {
                if c.contains([';', '"', '\n', '\r']) {
                    Cow::Owned(format!("\"{}\"", c.replace('"', "\"\"")))
                } else {
                    Cow::Borrowed(c.as_str())
                }
            })
            .collect();
        writeln!(w, "{}", cells.join(";"))?;
    }
    w.flush()
}

fn write_json(path: &Path, data: &[serde_json::Value]) -> io::Result<()> {
    let mut w = BufWriter::new(File::create(path)?);
    serde_json::to_writer_pretty(&mut w, data)?;
    w.flush()
}
