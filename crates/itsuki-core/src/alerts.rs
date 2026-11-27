//! Avisos de espacio: unidades casi llenas y a qué ritmo se llenan.
//!
//! El ritmo se calcula comparando el espacio usado ahora con el de la foto más
//! antigua del historial de los últimos 30 días (de al menos 6 horas), así que
//! mejora a medida que se guardan fotos con `itsuki crecimiento`.

use serde::Serialize;

use crate::history::{History, Result};
use crate::system::Volume;

const DAY: i64 = 86_400;
const GB: u64 = 1 << 30;

/// Gravedad del estado de una unidad.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Level {
    Ok,
    Warning,
    Critical,
}

impl Level {
    pub fn label(self) -> &'static str {
        match self {
            Level::Ok => "Bien",
            Level::Warning => "Aviso",
            Level::Critical => "Crítico",
        }
    }
}

/// Umbrales de los avisos. Se avisa cuando el espacio libre baja del mayor
/// entre el porcentaje y la cantidad fija (en discos grandes manda el
/// porcentaje; en los pequeños, la cantidad).
#[derive(Clone, Copy, Debug, Serialize)]
pub struct AlertRules {
    pub warn_pct: f64,
    pub warn_bytes: u64,
    pub critical_pct: f64,
    pub critical_bytes: u64,
    /// Avisar si, al ritmo actual, la unidad se llena antes de estos días.
    pub soon_days: f64,
}

impl Default for AlertRules {
    fn default() -> Self {
        AlertRules { warn_pct: 10.0, warn_bytes: 20 * GB, critical_pct: 5.0, critical_bytes: 5 * GB, soon_days: 30.0 }
    }
}

/// Estado de una unidad.
#[derive(Clone, Debug, Serialize)]
pub struct VolumeStatus {
    pub volume: Volume,
    pub level: Level,
    /// Cambio del espacio usado en bytes por día (positivo si se llena).
    pub growth_per_day: Option<f64>,
    /// Días hasta llenarse si sigue el mismo ritmo.
    pub days_to_full: Option<f64>,
    /// Momento de la foto con la que se calculó el ritmo.
    pub since: Option<i64>,
    /// Motivos del aviso, en texto.
    pub reasons: Vec<String>,
}

/// Calcula el estado de cada unidad.
pub fn volume_status(
    volumes: &[Volume],
    history: Option<&History>,
    rules: &AlertRules,
    now: i64,
) -> Result<Vec<VolumeStatus>> {
    let mut out = Vec::with_capacity(volumes.len());
    for v in volumes {
        let mut st = VolumeStatus {
            volume: v.clone(),
            level: Level::Ok,
            growth_per_day: None,
            days_to_full: None,
            since: None,
            reasons: Vec::new(),
        };
        if v.total == 0 {
            out.push(st);
            continue;
        }
        let free = v.free as f64;
        let total = v.total as f64;
        let critical = (rules.critical_bytes as f64).max(total * rules.critical_pct / 100.0);
        let warning = (rules.warn_bytes as f64).max(total * rules.warn_pct / 100.0);
        let pct = free * 100.0 / total;
        if free < critical {
            st.level = Level::Critical;
            st.reasons.push(format!("solo queda un {} libre", fmt_pct(pct)));
        } else if free < warning {
            st.level = Level::Warning;
            st.reasons.push(format!("queda un {} libre", fmt_pct(pct)));
        }

        if let Some(h) = history {
            if let Some(s) = h.oldest_on_volume(&v.root, now - 30 * DAY, now - 6 * 3600)? {
                if let Some(used_then) = s.volume_used() {
                    let days = (now - s.taken_at) as f64 / DAY as f64;
                    let rate = (v.used() as f64 - used_then as f64) / days;
                    st.growth_per_day = Some(rate);
                    st.since = Some(s.taken_at);
                    if rate > 0.0 {
                        let left = free / rate;
                        st.days_to_full = Some(left);
                        if left < rules.soon_days {
                            st.level = st.level.max(Level::Warning);
                            st.reasons.push(format!("a este ritmo se llena en {left:.0} días"));
                        }
                    }
                }
            }
        }
        out.push(st);
    }
    Ok(out)
}

fn fmt_pct(p: f64) -> String {
    format!("{p:.1} %").replace('.', ",")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::system::DriveKind;
    use crate::tree::tests::tree_from;

    fn vol(letter: &str, total: u64, free: u64) -> Volume {
        Volume {
            root: PathBuf::from(format!("{letter}:\\")),
            label: String::new(),
            filesystem: "NTFS".into(),
            total,
            free,
            kind: DriveKind::Fixed,
            seek_penalty: None,
        }
    }

    #[test]
    fn niveles_por_espacio_libre() {
        let r = AlertRules::default();
        let st = volume_status(
            &[vol("A", 2000 * GB, 1000 * GB), vol("B", 2000 * GB, 150 * GB), vol("C", 100 * GB, 4 * GB)],
            None,
            &r,
            0,
        )
        .unwrap();
        let levels: Vec<Level> = st.iter().map(|s| s.level).collect();
        assert_eq!(levels, vec![Level::Ok, Level::Warning, Level::Critical]);
        assert!(st[1].reasons[0].contains("7,5 %"));
    }

    #[test]
    fn ritmo_de_llenado_con_historial() {
        let now = 100 * DAY;
        let mut h = History::open_in_memory().unwrap();
        let t = tree_from(&[(0, "x", false, 10)], None);
        // Hace 10 días había 300 GB libres; ahora quedan 200 GB: 10 GB al día.
        h.save(&t, Some(&vol("R", 1000 * GB, 300 * GB)), 1, now - 10 * DAY).unwrap();
        let st = volume_status(&[vol("R", 1000 * GB, 200 * GB)], Some(&h), &AlertRules::default(), now).unwrap();
        let rate = st[0].growth_per_day.unwrap();
        assert!((rate - 10.0 * GB as f64).abs() < 1.0);
        assert!((st[0].days_to_full.unwrap() - 20.0).abs() < 0.01);
        assert_eq!(st[0].level, Level::Warning, "se llena en menos de 30 días");
        assert!(st[0].reasons.iter().any(|r| r.contains("20 días")));
    }
}
