//! Comandos `limpieza` y `duplicados`.

use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use clap::Args;
use itsuki_core::clean::{
    DeleteReport, JunkCategory, JunkContext, JunkItem, JunkReport, Safety, clean_item, item_path,
};
use itsuki_core::format::{self, count, size};
use itsuki_core::{DupOptions, DupProgress, Tree, find_duplicates, find_junk, system};
use serde_json::json;

use super::{Ctx, ExportArgs, Res, RutaArgs, parse_size_arg, write_csv, write_json};
use crate::ui::{self, Cell, Style, Table};

#[derive(Args)]
pub(crate) struct LimpiezaArgs {
    #[command(flatten)]
    ruta: RutaArgs,
    /// Muestra todas las ubicaciones de cada categoría (y los errores al limpiar)
    #[arg(long)]
    detalle: bool,
    /// Qué limpiar: «seguro», «regenerable» (incluye lo seguro) o categorías separadas por comas:
    /// temporales, navegadores, aplicaciones, sombreadores, windows-update, volcados,
    /// dependencias, compilaciones, paquetes, juegos, papelera, descargas
    #[arg(long, value_name = "SELECCIÓN", value_delimiter = ',', num_args = 1..)]
    limpiar: Vec<String>,
    /// Envía a la Papelera en vez de borrar de forma permanente
    #[arg(long)]
    papelera: bool,
    /// No pedir confirmación
    #[arg(long)]
    si: bool,
}

#[derive(Args)]
pub(crate) struct DuplicadosArgs {
    /// Carpeta o unidad a revisar (por defecto, tus carpetas personales: Escritorio,
    /// Documentos, Descargas, Imágenes, Vídeos, Música y OneDrive)
    ruta: Option<PathBuf>,
    /// Tamaño mínimo de los archivos (ej.: 100KB, 1MB, 50MB)
    #[arg(long, default_value = "1MB", value_parser = parse_size_arg)]
    min: u64,
    /// Cuántos grupos mostrar
    #[arg(long, default_value_t = 30)]
    top: usize,
    /// Incluir Windows, programas, bibliotecas de juegos, AppData, node_modules y carpetas ocultas (.git…)
    #[arg(long)]
    incluir_sistema: bool,
    #[command(flatten)]
    export: ExportArgs,
}

fn safety_cell(s: Safety) -> Cell {
    let paint: fn(Style, &str) -> String = match s {
        Safety::Safe => Style::green,
        Safety::Regenerable => Style::cyan,
        Safety::Review => Style::yellow,
        Safety::Advice => Style::dim,
    };
    Cell::styled(s.label(), paint)
}

/// Hallazgos que corresponden a la selección del usuario.
fn select<'r>(report: &'r JunkReport, wanted: &[String]) -> Result<Vec<&'r JunkItem>, String> {
    let mut level: Option<Safety> = None;
    let mut categories = Vec::new();
    for w in wanted.iter().map(|w| w.trim().to_lowercase()).filter(|w| !w.is_empty()) {
        match w.as_str() {
            "seguro" => level = level.max(Some(Safety::Safe)),
            "regenerable" => level = level.max(Some(Safety::Regenerable)),
            id => match JunkCategory::from_id(id) {
                Some(c) => categories.push(c),
                None => {
                    let ids: Vec<&str> =
                        JunkCategory::ALL.iter().filter(|c| **c != JunkCategory::Advice).map(|c| c.id()).collect();
                    return Err(format!(
                        "no conozco «{w}»: usa seguro, regenerable o una categoría ({})",
                        ids.join(", ")
                    ));
                }
            },
        }
    }
    Ok(report
        .items
        .iter()
        .filter(|i| i.safety != Safety::Advice)
        .filter(|i| level.is_some_and(|l| i.safety <= l) || categories.contains(&i.category))
        .collect())
}

impl Ctx<'_> {
    pub(crate) fn limpieza(&self, a: &LimpiezaArgs) -> Res {
        let st = self.style;
        let tree = self.scan(&a.ruta, false)?;
        self.header(&tree);
        let report = find_junk(&tree, &JunkContext::from_env());
        if report.items.is_empty() {
            println!("  No se encontró basura conocida.");
            return Ok(());
        }

        println!("  {}", st.title("Basura encontrada"));
        let mut t = Table::new(&[
            ("Liberable", true),
            ("Ubicaciones", true),
            ("Archivos", true),
            ("Nivel", false),
            ("Categoría", false),
        ]);
        for c in report.by_category() {
            let id =
                if c.category == JunkCategory::Advice { String::new() } else { format!("  ({})", c.category.id()) };
            t.row(vec![
                Cell::styled(size(c.bytes), Style::bold),
                Cell::plain(count(c.items as u64)),
                Cell::plain(count(c.files)),
                safety_cell(c.safety),
                Cell::plain(format!("{}{id}", c.category.label())),
            ]);
        }
        t.print(st);
        println!(
            "\n  {} {} · {} {} · {} {}",
            st.green("Seguro:"),
            st.bold(&size(report.total(Safety::Safe))),
            st.cyan("regenerable:"),
            st.bold(&size(report.total(Safety::Regenerable))),
            st.yellow("revisar:"),
            st.bold(&size(report.total(Safety::Review)))
        );
        println!(
            "  {}",
            st.dim("Seguro: se regenera solo · Regenerable: se vuelve a crear o descargar (cuesta tiempo) · Revisar: puede ser tuyo")
        );

        self.print_details(&tree, &report, a.detalle);

        if a.limpiar.is_empty() {
            println!(
                "\n  {}",
                st.dim("Para limpiar: itsuki limpieza --limpiar seguro   (o regenerable, o categorías como temporales,papelera)")
            );
            return Ok(());
        }
        let selected = select(&report, &a.limpiar)?;
        if selected.is_empty() {
            println!("\n  Nada que limpiar con esa selección.");
            return Ok(());
        }
        self.run_cleanup(&tree, &selected, a)
    }

    fn print_details(&self, tree: &Tree, report: &JunkReport, all: bool) {
        let st = self.style;
        let limit = if all { usize::MAX } else { 5 };
        for cat in JunkCategory::ALL {
            let items: Vec<&JunkItem> = report.items.iter().filter(|i| i.category == cat).collect();
            if items.is_empty() {
                continue;
            }
            println!("\n  {}", st.title(cat.label()));
            if cat == JunkCategory::Advice {
                for i in &items {
                    let path = item_path(tree, i).map(|p| p.display().to_string()).unwrap_or_default();
                    println!("  {}  {}  {}", st.bold(&size(i.bytes)), i.label, st.dim(&path));
                    if let itsuki_core::clean::Action::Advice(text) = &i.action {
                        println!("      {text}");
                    }
                }
                continue;
            }
            let mut t = Table::new(&[("Liberable", true), ("Nivel", false), ("Qué es", false), ("Ubicación", false)]);
            for i in items.iter().take(limit) {
                let mut path = item_path(tree, i).map(|p| p.display().to_string()).unwrap_or_default();
                if i.nodes.len() > 1 {
                    path.push_str(&format!(" y {} más", i.nodes.len() - 1));
                }
                let label = if i.needs_admin { format!("{} (admin)", i.label) } else { i.label.to_string() };
                t.row(vec![
                    Cell::styled(size(i.bytes), Style::bold),
                    safety_cell(i.safety),
                    Cell::plain(label),
                    Cell::plain(path),
                ]);
            }
            t.print(st);
            if items.len() > limit {
                let rest: u64 = items.iter().skip(limit).map(|i| i.bytes).sum();
                println!("  {}", st.dim(&format!("… y {} ubicaciones más ({})", items.len() - limit, size(rest))));
            }
        }
    }

    fn run_cleanup(&self, tree: &Tree, selected: &[&JunkItem], a: &LimpiezaArgs) -> Res {
        let st = self.style;
        let total: u64 = selected.iter().map(|i| i.bytes).sum();
        println!(
            "\n  {} {} ubicaciones · {}",
            st.title("Se va a limpiar:"),
            count(selected.len() as u64),
            st.bold(&size(total))
        );
        let mut cats: Vec<JunkCategory> = selected.iter().map(|i| i.category).collect();
        cats.dedup();
        for c in cats {
            let bytes: u64 = selected.iter().filter(|i| i.category == c).map(|i| i.bytes).sum();
            println!("    {}  {}", st.bold(&size(bytes)), c.label());
        }
        if selected.iter().any(|i| i.needs_admin) && !system::is_elevated() {
            println!("  {}", st.yellow("⚠ Algunas ubicaciones necesitan administrador: sin permisos se saltarán."));
        }
        let how = if a.papelera {
            "Se enviará a la Papelera (el espacio se libera al vaciarla)."
        } else {
            "Se borrará de forma PERMANENTE (no pasa por la Papelera). Lo que esté en uso se salta."
        };
        println!("  {}", st.yellow(how));
        if !a.si {
            print!("  ¿Continuar? [s/N] ");
            io::stdout().flush()?;
            let mut answer = String::new();
            io::stdin().read_line(&mut answer)?;
            if !matches!(answer.trim().to_lowercase().as_str(), "s" | "si" | "sí" | "y" | "yes") {
                println!("  Cancelado: no se borró nada.");
                return Ok(());
            }
        }

        let mut done = DeleteReport::default();
        let mut failed = Vec::new();
        for item in selected {
            match clean_item(tree, item, a.papelera) {
                Ok(r) => done.merge(r),
                Err(e) => failed.push(format!("{}: {e}", item.label)),
            }
        }
        println!(
            "\n  {} {} en {} archivos",
            st.green(if a.papelera { "✓ Enviado a la Papelera:" } else { "✓ Liberado:" }),
            st.bold(&size(done.bytes)),
            count(done.files)
        );
        if done.skipped > 0 || !failed.is_empty() {
            println!(
                "  {}",
                st.yellow(&format!(
                    "{} elementos omitidos (en uso o sin permiso){}",
                    count(done.skipped + failed.len() as u64),
                    if a.detalle { ":" } else { ". Usa --detalle para verlos." }
                ))
            );
            if a.detalle {
                for e in failed.iter().chain(&done.errors) {
                    println!("    {}", st.dim(e));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn duplicados(&self, a: &DuplicadosArgs) -> Res {
        let st = self.style;
        let profile = std::env::var_os("USERPROFILE").map(PathBuf::from);
        let personal = a.ruta.is_none();
        let root = a.ruta.clone().or_else(|| profile.clone());
        let tree = self.scan(&RutaArgs { ruta: root }, false)?;
        self.header(&tree);

        // Sin ruta: solo las carpetas personales, donde los duplicados son tuyos.
        let mut include_roots = Vec::new();
        if let (true, Some(p)) = (personal, &profile) {
            for name in ["Desktop", "Documents", "Downloads", "Pictures", "Videos", "Music"] {
                include_roots.push(p.join(name));
            }
            if let Ok(entries) = std::fs::read_dir(p) {
                for e in entries.flatten() {
                    if e.file_name().to_string_lossy().starts_with("OneDrive") {
                        include_roots.push(e.path());
                    }
                }
            }
        }
        let opts = DupOptions { min_size: a.min, include_system: a.incluir_sistema, include_roots, threads: 0 };
        let progress = DupProgress::new();
        let show = !self.global.silencioso && io::stderr().is_terminal();
        let done = AtomicBool::new(false);
        let report = std::thread::scope(|s| {
            if show {
                s.spawn(|| {
                    ui::status_loop(&done, || {
                        let (files, bytes) = progress.snapshot();
                        format!("Comparando contenido… {} archivos · {} leídos", count(files), size(bytes))
                    })
                });
            }
            let r = find_duplicates(&tree, &opts, &progress);
            done.store(true, Ordering::Relaxed);
            r
        })?;

        if report.groups.is_empty() {
            println!("  No hay archivos duplicados de {} o más.", size(a.min));
            return Ok(());
        }
        println!("  {}", st.title("Archivos duplicados"));
        let mut t = Table::new(&[("Recuperable", true), ("Copias", true), ("Tamaño", true), ("Archivos", false)]);
        for g in report.groups.iter().take(a.top) {
            for (i, &id) in g.files.iter().enumerate() {
                let path = tree.path(id).display().to_string();
                if i == 0 {
                    t.row(vec![
                        Cell::styled(size(g.wasted()), Style::bold),
                        Cell::plain(count(g.files.len() as u64)),
                        Cell::plain(size(g.size)),
                        Cell::plain(path),
                    ]);
                } else {
                    t.row(vec![Cell::plain(""), Cell::plain(""), Cell::plain(""), Cell::styled(path, Style::dim)]);
                }
            }
        }
        t.print(st);
        let mut footer = format!(
            "{} grupos · {} recuperables dejando una copia de cada uno · leídos {} de {} archivos en {}",
            count(report.groups.len() as u64),
            size(report.wasted()),
            size(report.read_bytes),
            count(report.read_files),
            format::seconds(report.elapsed)
        );
        if report.groups.len() > a.top {
            footer.push_str(&format!(" · mostrando {}", a.top));
        }
        println!("\n  {}", st.dim(&footer));
        if report.errors > 0 {
            println!(
                "  {}",
                st.yellow(&format!("{} archivos no se pudieron leer (en uso o sin permiso)", count(report.errors)))
            );
        }
        if personal {
            println!(
                "  {}",
                st.dim(
                    "Se revisaron tus carpetas personales (Escritorio, Documentos, Descargas, Imágenes, Vídeos, \
                     Música, OneDrive). Indica una ruta para revisar otra."
                )
            );
        }
        if !a.incluir_sistema {
            println!(
                "  {}",
                st.dim("Se omiten Windows, programas, bibliotecas de juegos, AppData, node_modules y carpetas ocultas (--incluir-sistema).")
            );
        }

        if let Some(p) = &a.export.csv {
            let rows = report.groups.iter().enumerate().flat_map(|(gi, g)| {
                let tree = &tree;
                g.files.iter().map(move |&id| {
                    vec![
                        (gi + 1).to_string(),
                        tree.path(id).display().to_string(),
                        g.size.to_string(),
                        g.alloc.to_string(),
                    ]
                })
            });
            write_csv(p, &["grupo", "ruta", "tamano_bytes", "en_disco_bytes"], rows)?;
            println!("  {} {}", st.green("CSV guardado en"), p.display());
        }
        if let Some(p) = &a.export.json {
            let data: Vec<_> = report
                .groups
                .iter()
                .map(|g| {
                    json!({
                        "tamano": g.size,
                        "en_disco": g.alloc,
                        "recuperable": g.wasted(),
                        "rutas": g.files.iter().map(|&id| tree.path(id)).collect::<Vec<_>>(),
                    })
                })
                .collect();
            write_json(p, &data)?;
            println!("  {} {}", st.green("JSON guardado en"), p.display());
        }
        Ok(())
    }
}
