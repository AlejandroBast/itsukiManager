//! Formato y lectura de tamaños, cantidades y fechas (convenciones en español).

use std::time::{SystemTime, UNIX_EPOCH};

const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];

/// Formatea bytes con unidades binarias, igual que el Explorador de Windows:
/// `512 B`, `1,50 GB`, `332,1 GB`.
pub fn size(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    let text = if value < 10.0 { format!("{value:.2}") } else { format!("{value:.1}") };
    format!("{} {}", text.replace('.', ","), UNITS[unit])
}

/// Lee tamaños como `1GB`, `500 MB`, `1,5g`, `2TiB` o `1024` (bytes).
/// Usa unidades binarias (1 GB = 1024³ bytes).
pub fn parse_size(text: &str) -> Option<u64> {
    let t = text.trim().replace(',', ".").to_ascii_lowercase();
    let split = t.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(t.len());
    let (number, unit) = t.split_at(split);
    let value: f64 = number.parse().ok()?;
    let multiplier: u64 = match unit.trim() {
        "" | "b" => 1,
        "k" | "kb" | "kib" => 1 << 10,
        "m" | "mb" | "mib" => 1 << 20,
        "g" | "gb" | "gib" => 1 << 30,
        "t" | "tb" | "tib" => 1 << 40,
        _ => return None,
    };
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    Some((value * multiplier as f64).round() as u64)
}

/// Cantidad con separador de miles: `596.169`.
pub fn count(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push('.');
        }
        out.push(ch);
    }
    out
}

/// Porcentaje con un decimal: `52,5%`.
pub fn percent(part: u64, total: u64) -> String {
    if total == 0 {
        return "0,0%".into();
    }
    format!("{:.1}%", part as f64 * 100.0 / total as f64).replace('.', ",")
}

/// Duración en segundos con dos decimales: `2,31 s`.
pub fn seconds(d: std::time::Duration) -> String {
    format!("{:.2} s", d.as_secs_f64()).replace('.', ",")
}

/// Fecha `AAAA-MM-DD` a partir de segundos Unix (UTC). `-` si es desconocida.
pub fn date(unix_secs: i64) -> String {
    if unix_secs <= 0 {
        return "-".into();
    }
    let (y, m, d) = civil_from_days(unix_secs.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// Segundos Unix actuales.
pub fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Días desde 1970-01-01 a fecha civil (algoritmo de Howard Hinnant).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formatea_tamanos() {
        assert_eq!(size(0), "0 B");
        assert_eq!(size(1023), "1023 B");
        assert_eq!(size(1536), "1,50 KB");
        assert_eq!(size(1 << 30), "1,00 GB");
        assert_eq!(size(332 * (1 << 30) + (1 << 27)), "332,1 GB");
    }

    #[test]
    fn lee_tamanos() {
        assert_eq!(parse_size("1GB"), Some(1 << 30));
        assert_eq!(parse_size("1,5 gb"), Some(3 << 29));
        assert_eq!(parse_size("500MB"), Some(500 << 20));
        assert_eq!(parse_size("2TiB"), Some(2 << 40));
        assert_eq!(parse_size("1024"), Some(1024));
        assert_eq!(parse_size("0"), Some(0));
        assert_eq!(parse_size("abc"), None);
        assert_eq!(parse_size("5 xb"), None);
    }

    #[test]
    fn formatea_cantidades_y_porcentajes() {
        assert_eq!(count(0), "0");
        assert_eq!(count(999), "999");
        assert_eq!(count(1000), "1.000");
        assert_eq!(count(596_169), "596.169");
        assert_eq!(count(1_234_567_890), "1.234.567.890");
        assert_eq!(percent(1, 3), "33,3%");
        assert_eq!(percent(5, 0), "0,0%");
    }

    #[test]
    fn formatea_fechas() {
        assert_eq!(date(0), "-");
        assert_eq!(date(86_400), "1970-01-02");
        assert_eq!(date(951_782_400), "2000-02-29");
        assert_eq!(date(1_795_651_200), "2026-11-26");
    }
}
