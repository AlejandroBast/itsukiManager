//! Comandos `historial`, `crecimiento` y `avisos`.

use std::path::PathBuf;

use clap::{Args, Subcommand};
use itsuki_core::alerts::{AlertRules, Level, volume_status};
use itsuki_core::format::{self, count, size};
use itsuki_core::history::{DEFAULT_MIN_FOLDER, History, compare, folder_rows};
use itsuki_core::system::{self, DriveKind};

use super::{Ctx, Res, RutaArgs, parse_size_arg, signed_size};
use crate::ui::{Cell, Style, Table};

#[derive(Args)]
pub(crate) struct HistorialArgs {
    #[command(subcommand)]
    accion: Option<HistorialCmd>,
}

#[derive(Subcommand)]
enum HistorialCmd {
    /// Lista las fotos guardadas (es lo que hace «itsuki historial» sin más)
    Lista {
        /// Solo las fotos de esta ruta
        ruta: Option<PathBuf>,
    },
    /// Escanea y guarda una foto ahora
    Guardar(RutaArgs),
    /// Borra una foto por su número
    Borrar { numero: i64 },
}

#[derive(Args)]
pub(crate) struct CrecimientoArgs {
    #[command(flatten)]
    ruta: RutaArgs,
    /// Foto con la que comparar: número (12), antigüedad (24h, 7d, 2s, 1m) o fecha (2026-11-20).
    /// Por defecto, la foto anterior de la misma ruta
    #[arg(long, value_name = "FOTO")]
    desde: Option<String>,
    /// Cambio mínimo para mostrar una carpeta
    #[arg(long, default_value = "100MB", value_parser = parse_size_arg)]
    min: u64,
    /// Cuántas carpetas mostrar
    #[arg(long, default_value_t = 20)]
    top: usize,
    /// Muestra también las carpetas padre, no solo las que explican el cambio
    #[arg(long)]
    todas: bool,
    /// No guardar este escaneo en el historial
    #[arg(long)]
    sin_guardar: bool,
}

#[derive(Args)]
pub(crate) struct AvisosArgs {
    /// Espacio libre mínimo antes de avisar: porcentaje (10%) o tamaño (20GB)
    #[arg(long, value_name = "LÍMITE")]
    min_libre: Option<String>,
}

/// Foto de referencia pedida con `--desde`.
enum Since {
    Id(i64),
    Before(i64),
}

fn parse_since(text: &str, now: i64) -> Result<Since, String> {
    let t = text.trim().trim_start_matches('#');
    if let Ok(id) = t.parse::<i64>() {
        return Ok(Since::Id(id));
    }
    if let Some(end) = format::parse_date_end(t) {
        return Ok(Since::Before(end));
    }
    let split = t.find(|c: char| !c.is_ascii_digit()).unwrap_or(t.len());
    let (num, unit) = t.split_at(split);
    if let (Ok(n), false) = (num.parse::<i64>(), num.is_empty()) {
        let secs = match unit.trim().to_lowercase().as_str() {
            "h" => 3600,
            "d" => 86_400,
            "s" | "sem" | "w" => 7 * 86_400,
            "m" | "mes" | "meses" => 30 * 86_400,
            _ => return Err(format!("unidad desconocida en «{text}»: usa h, d, s (semanas) o m (meses)")),
        };
        return Ok(Since::Before(now - n * secs));
    }
    Err(format!("no entiendo «{text}»: usa un número de foto, 24h, 7d, 2s, 1m o una fecha 2026-11-20"))
}

fn level_cell(level: Level) -> Cell {
    let paint: fn(Style, &str) -> String = match level {
        Level::Ok => Style::green,
        Level::Warning => Style::yellow,
        Level::Critical => Style::red,
    };
    Cell::styled(level.label(), paint)
}

impl Ctx<'_> {
    pub(crate) fn historial(&self, a: &HistorialArgs) -> Res {
        match &a.accion {
            None => self.historial_lista(None),
            Some(HistorialCmd::Lista { ruta }) => self.historial_lista(ruta.as_deref()),
            Some(HistorialCmd::Guardar(r)) => {
                let tree = self.scan(r, false)?;
                self.header(&tree);
                let mut h = History::open_default()?;
                let vol = system::volume_of(tree.root_path());
                let id = h.save(&tree, vol.as_ref(), DEFAULT_MIN_FOLDER, format::now_unix())?;
                let folders = h.folders(id)?.len();
                println!(
                    "  {} foto #{id} con {} carpetas de 100 MB o más.",
                    self.style.green("✓ Guardada la"),
                    count(folders as u64)
                );
                Ok(())
            }
            Some(HistorialCmd::Borrar { numero }) => {
                let mut h = History::open_default()?;
                if h.delete(*numero)? {
                    println!("  {} foto #{numero}.", self.style.green("✓ Borrada la"));
                    Ok(())
                } else {
                    Err(format!("no existe la foto #{numero}").into())
                }
            }
        }
    }

    fn historial_lista(&self, ruta: Option<&std::path::Path>) -> Res {
        let st = self.style;
        let h = History::open_default()?;
        let list = h.list(ruta)?;
        println!("{}", st.title("Historial de escaneos"));
        if list.is_empty() {
            println!("  Todavía no hay fotos. Guarda una con «itsuki crecimiento» o «itsuki historial guardar».");
        } else {
            let now = format::now_unix();
            let mut t = Table::new(&[
                ("Nº", true),
                ("Fecha", false),
                ("Cuándo", false),
                ("En disco", true),
                ("Archivos", true),
                ("Libre", true),
                ("Motor", false),
                ("Ruta", false),
            ]);
            for s in &list {
                t.row(vec![
                    Cell::styled(format!("#{}", s.id), Style::bold),
                    Cell::plain(format::datetime(s.taken_at)),
                    Cell::styled(format::ago(now - s.taken_at), Style::dim),
                    Cell::plain(size(s.alloc)),
                    Cell::plain(count(s.files)),
                    Cell::plain(s.volume_free.map_or_else(|| "-".into(), size)),
                    Cell::plain(s.engine.clone()),
                    Cell::plain(s.root.clone()),
                ]);
            }
            t.print(st);
        }
        println!(
            "\n  {}",
            st.dim(&format!("Base de datos: {}", History::default_dir().join("historial.db").display()))
        );
        Ok(())
    }

    pub(crate) fn crecimiento(&self, a: &CrecimientoArgs) -> Res {
        let st = self.style;
        // Se valida --desde antes de escanear para no esperar en balde.
        let since = a.desde.as_deref().map(|t| parse_since(t, format::now_unix())).transpose()?;
        let tree = self.scan(&a.ruta, false)?;
        self.header(&tree);
        let mut h = History::open_default()?;
        let now = format::now_unix();
        let vol = system::volume_of(tree.root_path());
        let current = folder_rows(&tree, DEFAULT_MIN_FOLDER);
        let new_id = if a.sin_guardar { None } else { Some(h.save(&tree, vol.as_ref(), DEFAULT_MIN_FOLDER, now)?) };

        let reference = match since {
            None => h.latest_before(tree.root_path(), now, new_id)?,
            Some(Since::Id(id)) => h.get(id)?,
            Some(Since::Before(t)) => h.latest_before(tree.root_path(), t, new_id)?,
        };
        let Some(old) = reference else {
            match (&a.desde, new_id) {
                (Some(d), _) => println!("  No hay ninguna foto de {} para «{d}».", tree.root_path().display()),
                (None, Some(id)) => println!(
                    "  {} foto #{id}: es la primera de {}. Vuelve a ejecutar «itsuki crecimiento» más adelante \
                     (por ejemplo, mañana) para ver qué crece.",
                    st.green("✓ Guardada la"),
                    tree.root_path().display()
                ),
                (None, None) => println!("  No hay fotos anteriores con las que comparar."),
            }
            return Ok(());
        };

        let changes = compare(&h.folders(old.id)?, &current, a.min, !a.todas);
        let root = tree.node(itsuki_core::ROOT);
        println!(
            "  {} #{} del {} ({})",
            st.title("Comparado con la foto"),
            old.id,
            format::datetime(old.taken_at),
            format::ago(now - old.taken_at)
        );
        println!(
            "  Espacio escaneado: {} → {} ({})",
            size(old.alloc),
            st.bold(&size(root.alloc)),
            st.bold(&signed_size(i128::from(root.alloc) - i128::from(old.alloc)))
        );
        if let (Some(v), Some(used_then)) = (&vol, old.volume_used()) {
            println!(
                "  Unidad {}: usado {} → {} ({}) · libre {}",
                v.root.display(),
                size(used_then),
                size(v.used()),
                signed_size(i128::from(v.used()) - i128::from(used_then)),
                st.green(&size(v.free))
            );
        }
        if let Some(id) = new_id {
            println!("  {}", st.dim(&format!("Esta foto se guardó como #{id}.")));
        }
        println!();
        if changes.is_empty() {
            println!("  Ninguna carpeta cambió {} o más.", size(a.min));
            return Ok(());
        }
        let mut t = Table::new(&[("Cambio", true), ("Antes", true), ("Ahora", true), ("Carpeta", false)]);
        for c in changes.iter().take(a.top) {
            let d = c.delta();
            let paint: fn(Style, &str) -> String = if d > 0 { Style::yellow } else { Style::green };
            let note = match (c.before, c.after) {
                (None, _) => "  (nueva o antes menor de 100 MB)",
                (_, None) => "  (ya no está o ahora mide menos de 100 MB)",
                _ => "",
            };
            t.row(vec![
                Cell::styled(signed_size(i128::from(d)), paint),
                Cell::plain(c.before.map_or_else(|| "-".into(), size)),
                Cell::plain(c.after.map_or_else(|| "-".into(), size)),
                Cell::plain(format!("{}{note}", c.path)),
            ]);
        }
        t.print(st);
        let grew: i64 = changes.iter().map(|c| c.delta()).filter(|d| *d > 0).sum();
        let shrank: i64 = changes.iter().map(|c| c.delta()).filter(|d| *d < 0).sum();
        let mut footer = format!(
            "{} carpetas cambiaron {} o más · crecieron {} · se redujeron {}",
            count(changes.len() as u64),
            size(a.min),
            signed_size(i128::from(grew)),
            signed_size(i128::from(shrank))
        );
        if changes.len() > a.top {
            footer.push_str(&format!(" · mostrando {}", a.top));
        }
        println!("\n  {}", st.dim(&footer));
        if !a.todas {
            println!(
                "  {}",
                st.dim("Se muestran las carpetas que explican el cambio; --todas incluye también sus padres.")
            );
        }
        Ok(())
    }

    pub(crate) fn avisos(&self, a: &AvisosArgs) -> Res {
        let st = self.style;
        let mut rules = AlertRules::default();
        if let Some(limit) = &a.min_libre {
            let text = limit.trim();
            if let Some(pct) = text.strip_suffix('%') {
                rules.warn_pct =
                    pct.trim().replace(',', ".").parse().map_err(|_| format!("porcentaje no válido: {text}"))?;
                rules.warn_bytes = 0;
            } else {
                rules.warn_bytes = parse_size_arg(text)?;
                rules.warn_pct = 0.0;
            }
        }
        let volumes: Vec<_> = system::volumes()
            .into_iter()
            .filter(|v| matches!(v.kind, DriveKind::Fixed | DriveKind::Removable) && v.total > 0)
            .collect();
        let history = History::open_default().ok();
        let now = format::now_unix();
        let status = volume_status(&volumes, history.as_ref(), &rules, now)?;

        println!("{}", st.title("Estado de las unidades"));
        let mut t = Table::new(&[
            ("Unidad", false),
            ("Libre", true),
            ("% libre", true),
            ("Estado", false),
            ("Tendencia", true),
            ("Se llena en", true),
            ("Motivo", false),
        ]);
        let mut trend_known = false;
        for s in &status {
            let v = &s.volume;
            trend_known |= s.growth_per_day.is_some();
            let trend =
                s.growth_per_day.map_or_else(|| "-".into(), |r| format!("{}/día", signed_size(r.round() as i128)));
            let full = s.days_to_full.map_or_else(
                || "-".into(),
                |d| if d > 3650.0 { "más de 10 años".into() } else { format!("~{d:.0} días") },
            );
            t.row(vec![
                Cell::styled(v.root.display().to_string(), Style::bold),
                Cell::plain(size(v.free)),
                Cell::plain(format::percent(v.free, v.total)),
                level_cell(s.level),
                Cell::plain(trend),
                Cell::plain(full),
                Cell::plain(s.reasons.join("; ")),
            ]);
        }
        t.print(st);
        if !trend_known {
            println!(
                "\n  {}",
                st.dim("Para ver a qué ritmo se llena cada unidad, guarda fotos con «itsuki crecimiento» (por ejemplo, una vez al día).")
            );
        }
        let alerts = status.iter().filter(|s| s.level != Level::Ok).count();
        if alerts > 0 {
            println!(
                "\n  {}",
                st.yellow(&format!("⚠ {alerts} unidad(es) necesitan atención: prueba «itsuki limpieza»."))
            );
            // Código de salida 1 para usarlo desde scripts y tareas programadas.
            std::process::exit(1);
        }
        println!("\n  {}", st.green("✓ Todas las unidades tienen espacio suficiente."));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lee_desde() {
        let now = 1_000_000;
        assert!(matches!(parse_since("12", now), Ok(Since::Id(12))));
        assert!(matches!(parse_since("#7", now), Ok(Since::Id(7))));
        assert!(matches!(parse_since("24h", now), Ok(Since::Before(t)) if t == now - 86_400));
        assert!(matches!(parse_since("7d", now), Ok(Since::Before(t)) if t == now - 7 * 86_400));
        assert!(matches!(parse_since("2s", now), Ok(Since::Before(t)) if t == now - 14 * 86_400));
        assert!(matches!(parse_since("1m", now), Ok(Since::Before(t)) if t == now - 30 * 86_400));
        assert!(matches!(parse_since("2026-11-20", now), Ok(Since::Before(_))));
        assert!(parse_since("7x", now).is_err());
        assert!(parse_since("ayer", now).is_err());
    }
}
