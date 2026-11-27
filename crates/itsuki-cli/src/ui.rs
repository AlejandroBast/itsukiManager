//! Salida en consola: colores, tablas, barras y progreso.

use std::io::{self, IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use itsuki_core::format;
use itsuki_core::scan::ScanProgress;

/// Colores ANSI (se desactivan si la salida no es una consola o con NO_COLOR).
#[derive(Clone, Copy)]
pub struct Style {
    color: bool,
}

impl Style {
    pub fn detect() -> Style {
        let color = std::env::var_os("NO_COLOR").is_none() && io::stdout().is_terminal() && enable_vt();
        Style { color }
    }

    fn paint(self, code: &str, text: &str) -> String {
        if self.color { format!("\x1b[{code}m{text}\x1b[0m") } else { text.to_string() }
    }

    pub fn bold(self, t: &str) -> String {
        self.paint("1", t)
    }
    pub fn dim(self, t: &str) -> String {
        self.paint("2", t)
    }
    pub fn cyan(self, t: &str) -> String {
        self.paint("36", t)
    }
    pub fn green(self, t: &str) -> String {
        self.paint("32", t)
    }
    pub fn yellow(self, t: &str) -> String {
        self.paint("33", t)
    }
    pub fn red(self, t: &str) -> String {
        self.paint("31", t)
    }
    pub fn title(self, t: &str) -> String {
        self.paint("1;36", t)
    }
}

/// Activa las secuencias ANSI en la consola de Windows.
#[cfg(windows)]
fn enable_vt() -> bool {
    use windows_sys::Win32::System::Console::{
        ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode, GetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
        SetConsoleMode,
    };
    let mut ok = false;
    for (i, which) in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE].into_iter().enumerate() {
        // SAFETY: llamadas de consola con un handle estándar.
        unsafe {
            let h = GetStdHandle(which);
            let mut mode = 0;
            if GetConsoleMode(h, &mut mode) != 0
                && SetConsoleMode(h, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING) != 0
                && i == 0
            {
                ok = true;
            }
        }
    }
    ok
}

#[cfg(not(windows))]
fn enable_vt() -> bool {
    true
}

/// Barra proporcional de `width` caracteres con precisión de 1/8.
pub fn bar(fraction: f64, width: usize) -> String {
    const PARTS: [char; 8] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉'];
    let eighths = (fraction.clamp(0.0, 1.0) * width as f64 * 8.0).round() as usize;
    let full = (eighths / 8).min(width);
    let mut s = "█".repeat(full);
    if full < width {
        s.push(PARTS[eighths % 8]);
        s.push_str(&" ".repeat(width - full - 1));
    }
    s
}

/// Celda de tabla con color opcional.
pub struct Cell {
    text: String,
    paint: Option<fn(Style, &str) -> String>,
}

impl Cell {
    pub fn plain(text: impl Into<String>) -> Cell {
        Cell { text: text.into(), paint: None }
    }
    pub fn styled(text: impl Into<String>, paint: fn(Style, &str) -> String) -> Cell {
        Cell { text: text.into(), paint: Some(paint) }
    }
}

/// Tabla alineada. Las columnas marcadas como numéricas se alinean a la
/// derecha; la última columna (normalmente una ruta) no se rellena.
pub struct Table {
    headers: Vec<&'static str>,
    right: Vec<bool>,
    rows: Vec<Vec<Cell>>,
}

impl Table {
    pub fn new(columns: &[(&'static str, bool)]) -> Table {
        Table {
            headers: columns.iter().map(|c| c.0).collect(),
            right: columns.iter().map(|c| c.1).collect(),
            rows: Vec::new(),
        }
    }

    pub fn row(&mut self, cells: Vec<Cell>) {
        self.rows.push(cells);
    }

    pub fn print(&self, style: Style) {
        let n = self.headers.len();
        let mut widths: Vec<usize> = self.headers.iter().map(|h| h.chars().count()).collect();
        for r in &self.rows {
            for (i, c) in r.iter().enumerate().take(n) {
                widths[i] = widths[i].max(c.text.chars().count());
            }
        }
        let fit = |text: &str, i: usize| {
            let pad = widths[i].saturating_sub(text.chars().count());
            if i + 1 == n {
                text.to_string()
            } else if self.right[i] {
                format!("{}{text}", " ".repeat(pad))
            } else {
                format!("{text}{}", " ".repeat(pad))
            }
        };
        let header: Vec<String> = self.headers.iter().enumerate().map(|(i, h)| fit(h, i)).collect();
        println!("  {}", style.dim(&header.join("  ")));
        let mut out = io::stdout().lock();
        for r in &self.rows {
            let line: Vec<String> = r
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let t = fit(&c.text, i);
                    match c.paint {
                        Some(p) => p(style, &t),
                        None => t,
                    }
                })
                .collect();
            let _ = writeln!(out, "  {}", line.join("  "));
        }
    }
}

/// Muestra en stderr una línea de estado que se refresca hasta que `done`
/// sea verdadero, y la borra al terminar.
pub fn status_loop(done: &AtomicBool, line: impl Fn() -> String) {
    let mut err = io::stderr();
    let mut last_len = 0usize;
    let spinner = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
    let mut tick = 0usize;
    while !done.load(Ordering::Relaxed) {
        let text = format!("  {} {}", spinner[tick % spinner.len()], line());
        let len = text.chars().count();
        let _ = write!(err, "\r{text}{}", " ".repeat(last_len.saturating_sub(len)));
        let _ = err.flush();
        last_len = len;
        tick += 1;
        std::thread::sleep(Duration::from_millis(80));
    }
    let _ = write!(err, "\r{}\r", " ".repeat(last_len));
    let _ = err.flush();
}

/// Progreso del escaneo.
pub fn progress_loop(progress: &ScanProgress, done: &AtomicBool) {
    status_loop(done, || {
        let p = progress.snapshot();
        format!(
            "Escaneando… {} archivos · {} carpetas · {}",
            format::count(p.files),
            format::count(p.dirs),
            format::size(p.bytes)
        )
    });
}
#[cfg(test)]
mod tests {
    use super::bar;

    #[test]
    fn barras() {
        assert_eq!(bar(0.0, 4).chars().count(), 4);
        assert_eq!(bar(1.0, 4), "████");
        assert_eq!(bar(0.5, 4), "██  ");
        assert_eq!(bar(0.0625, 2), "▏ ");
        assert_eq!(bar(0.125, 2), "▎ ");
        assert_eq!(bar(2.0, 3), "███");
    }
}
