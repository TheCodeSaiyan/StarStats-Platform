//! Distribution helpers for reference stats: discover numeric leaf
//! paths in a metadata blob, and summarise a set of values as
//! quantiles. Pure + dependency-light so the server stats builder can
//! lean on tested logic.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Quantile summary of a numeric distribution. `n` is the sample size.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Quantiles {
    pub min: f64,
    pub p10: f64,
    pub p25: f64,
    pub p50: f64,
    pub p75: f64,
    pub p90: f64,
    pub max: f64,
    pub n: usize,
}

impl Quantiles {
    /// Summarise `values`. Returns `None` for an empty slice.
    pub fn from_values(values: &[f64]) -> Option<Quantiles> {
        if values.is_empty() {
            return None;
        }
        let mut v = values.to_vec();
        v.sort_by(|a, b| a.total_cmp(b));
        let q = |frac: f64| -> f64 {
            // Nearest-rank: index = ceil(frac * n) - 1, clamped.
            let idx = ((frac * v.len() as f64).ceil() as isize - 1).clamp(0, v.len() as isize - 1)
                as usize;
            v[idx]
        };
        Some(Quantiles {
            min: v[0],
            p10: q(0.10),
            p25: q(0.25),
            p50: q(0.50),
            p75: q(0.75),
            p90: q(0.90),
            max: v[v.len() - 1],
            n: v.len(),
        })
    }
}

/// Walk `metadata`, returning every numeric leaf as `(dotted_path,
/// value)`. Bools are excluded (they're flags, not measures). Arrays
/// are not descended — element shapes vary and per-index paths aren't
/// comparable across entities.
pub fn numeric_leaves(metadata: &Value) -> Vec<(String, f64)> {
    let mut out = Vec::new();
    walk(metadata, String::new(), &mut out);
    out
}

fn walk(v: &Value, prefix: String, out: &mut Vec<(String, f64)>) {
    match v {
        Value::Object(map) => {
            for (k, child) in map {
                let path = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                walk(child, path, out);
            }
        }
        Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                out.push((prefix, f));
            }
        }
        // Bools, strings, arrays, null: not numeric measures.
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn discovers_numeric_leaves_with_dotted_paths() {
        let meta = json!({
            "speed": { "scm": 262, "max": 1425.0 },
            "health": 11900,
            "is_spaceship": true,           // bool excluded
            "name": "Stalker",              // string excluded
            "weaponry": { "fixed": { "dps": 2359.6 } },
            "ports": [ { "x": 1 } ]          // arrays not descended
        });
        let mut leaves = numeric_leaves(&meta);
        leaves.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            leaves,
            vec![
                ("health".to_string(), 11900.0),
                ("speed.max".to_string(), 1425.0),
                ("speed.scm".to_string(), 262.0),
                ("weaponry.fixed.dps".to_string(), 2359.6),
            ]
        );
    }

    #[test]
    fn quantiles_basic() {
        let vals: Vec<f64> = (1..=100).map(|n| n as f64).collect();
        let q = Quantiles::from_values(&vals).unwrap();
        assert_eq!(q.n, 100);
        assert_eq!(q.min, 1.0);
        assert_eq!(q.max, 100.0);
        // Nearest-rank style; allow ±1 slack on interior quantiles.
        assert!((q.p50 - 50.0).abs() <= 1.0);
        assert!((q.p10 - 10.0).abs() <= 1.0);
        assert!((q.p90 - 90.0).abs() <= 1.0);
    }

    #[test]
    fn quantiles_empty_is_none() {
        assert!(Quantiles::from_values(&[]).is_none());
    }
}
