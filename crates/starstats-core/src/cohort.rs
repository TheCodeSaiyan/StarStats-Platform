//! Comparison cohorts — every "class" a reference entity belongs to, for
//! the KB "Compared to" baseline + comparison bulk-add. Four kinds:
//! `family` (coarse peer group), `type` (exact role/type/tier),
//! `make` (manufacturer), and `range` (preset attribute bands).
//!
//! Single source of truth shared by the server stats builder (buckets by
//! cohort key), the detail stamper (anchor's cohorts), and the client
//! (labels). Keys are stable, lowercase, hyphenated.

use crate::peer_group::peer_group;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cohort {
    pub key: String,
    pub kind: String, // "family" | "type" | "make" | "range"
    pub label: String,
}

/// Inclusive `min`, exclusive `max` (open-ended via `f64::INFINITY`).
struct Bucket {
    key: &'static str,
    label: &'static str,
    min: f64,
    max: f64,
}

struct RangeAttr {
    attr: &'static str,
    path: &'static str,
    buckets: &'static [Bucket],
}

const VEHICLE_RANGES: &[RangeAttr] = &[
    RangeAttr {
        attr: "cargo",
        path: "cargo_capacity",
        buckets: &[
            Bucket {
                key: "0-10",
                label: "Cargo <10 SCU",
                min: 0.0,
                max: 10.0,
            },
            Bucket {
                key: "10-100",
                label: "Cargo 10–100 SCU",
                min: 10.0,
                max: 100.0,
            },
            Bucket {
                key: "100-1000",
                label: "Cargo 100–1000 SCU",
                min: 100.0,
                max: 1000.0,
            },
            Bucket {
                key: "1000+",
                label: "Cargo 1000+ SCU",
                min: 1000.0,
                max: f64::INFINITY,
            },
        ],
    },
    RangeAttr {
        attr: "crew",
        path: "crew.max",
        buckets: &[
            Bucket {
                key: "1",
                label: "Solo crew",
                min: 1.0,
                max: 2.0,
            },
            Bucket {
                key: "2",
                label: "2 crew",
                min: 2.0,
                max: 3.0,
            },
            Bucket {
                key: "3-5",
                label: "3–5 crew",
                min: 3.0,
                max: 6.0,
            },
            Bucket {
                key: "6+",
                label: "6+ crew",
                min: 6.0,
                max: f64::INFINITY,
            },
        ],
    },
    RangeAttr {
        attr: "price",
        path: "msrp",
        buckets: &[
            Bucket {
                key: "0-75",
                label: "Pledge <$75",
                min: 0.0,
                max: 75.0,
            },
            Bucket {
                key: "75-150",
                label: "Pledge $75–150",
                min: 75.0,
                max: 150.0,
            },
            Bucket {
                key: "150-400",
                label: "Pledge $150–400",
                min: 150.0,
                max: 400.0,
            },
            Bucket {
                key: "400+",
                label: "Pledge $400+",
                min: 400.0,
                max: f64::INFINITY,
            },
        ],
    },
];

// Bucket boundaries chosen from observed live wiki value ranges:
// weapon effective_range 10–4400, size 1–5; item mass 0.1–480, size 1–3;
// location mission_count 4–648, size (diameter) 0.1–800k.
const WEAPON_RANGES: &[RangeAttr] = &[
    RangeAttr {
        attr: "size",
        path: "size",
        buckets: &[
            Bucket {
                key: "1",
                label: "Size 1",
                min: 1.0,
                max: 2.0,
            },
            Bucket {
                key: "2",
                label: "Size 2",
                min: 2.0,
                max: 3.0,
            },
            Bucket {
                key: "3+",
                label: "Size 3+",
                min: 3.0,
                max: f64::INFINITY,
            },
        ],
    },
    RangeAttr {
        attr: "range",
        path: "personal_weapon.effective_range",
        buckets: &[
            Bucket {
                key: "0-60",
                label: "Range <60 m",
                min: 0.0,
                max: 60.0,
            },
            Bucket {
                key: "60-150",
                label: "Range 60–150 m",
                min: 60.0,
                max: 150.0,
            },
            Bucket {
                key: "150+",
                label: "Range 150 m+",
                min: 150.0,
                max: f64::INFINITY,
            },
        ],
    },
];

const ITEM_RANGES: &[RangeAttr] = &[
    RangeAttr {
        attr: "size",
        path: "size",
        buckets: &[
            Bucket {
                key: "1",
                label: "Size 1",
                min: 1.0,
                max: 2.0,
            },
            Bucket {
                key: "2",
                label: "Size 2",
                min: 2.0,
                max: 3.0,
            },
            Bucket {
                key: "3",
                label: "Size 3",
                min: 3.0,
                max: f64::INFINITY,
            },
        ],
    },
    RangeAttr {
        attr: "mass",
        path: "mass",
        buckets: &[
            Bucket {
                key: "0-5",
                label: "Light (<5 kg)",
                min: 0.0,
                max: 5.0,
            },
            Bucket {
                key: "5-50",
                label: "Medium (5–50 kg)",
                min: 5.0,
                max: 50.0,
            },
            Bucket {
                key: "50+",
                label: "Heavy (50 kg+)",
                min: 50.0,
                max: f64::INFINITY,
            },
        ],
    },
];

const LOCATION_RANGES: &[RangeAttr] = &[
    RangeAttr {
        attr: "missions",
        path: "mission_count",
        buckets: &[
            Bucket {
                key: "0-50",
                label: "<50 missions",
                min: 0.0,
                max: 50.0,
            },
            Bucket {
                key: "50-200",
                label: "50–200 missions",
                min: 50.0,
                max: 200.0,
            },
            Bucket {
                key: "200+",
                label: "200+ missions",
                min: 200.0,
                max: f64::INFINITY,
            },
        ],
    },
    RangeAttr {
        attr: "diameter",
        path: "size",
        buckets: &[
            Bucket {
                key: "0-1000",
                label: "Small (<1 km)",
                min: 0.0,
                max: 1000.0,
            },
            Bucket {
                key: "1000-100000",
                label: "Medium",
                min: 1000.0,
                max: 100_000.0,
            },
            Bucket {
                key: "100000+",
                label: "Large",
                min: 100_000.0,
                max: f64::INFINITY,
            },
        ],
    },
];

fn ranges_for(category: &str) -> &'static [RangeAttr] {
    match category {
        "vehicle" => VEHICLE_RANGES,
        "weapon" => WEAPON_RANGES,
        "item" => ITEM_RANGES,
        "location" => LOCATION_RANGES,
        _ => &[],
    }
}

fn type_field(category: &str) -> &'static str {
    match category {
        "vehicle" => "role",
        "weapon" | "item" => "type_label",
        "location" => "tier",
        _ => "role",
    }
}

/// Every cohort `metadata` belongs to, for `category`. Order: family,
/// type, make, then ranges.
pub fn cohort_memberships(category: &str, metadata: &Value) -> Vec<Cohort> {
    let mut out = Vec::new();

    let fam = peer_group(category, metadata);
    out.push(Cohort {
        key: format!("family:{fam}"),
        kind: "family".into(),
        label: family_label(category, &fam),
    });

    if let Some(t) = str_field(metadata, type_field(category)) {
        let slug = slugify(&t);
        if !slug.is_empty() {
            out.push(Cohort {
                key: format!("type:{slug}"),
                kind: "type".into(),
                label: pluralish(&t),
            });
        }
    }

    if let Some(m) = manufacturer(metadata) {
        let slug = slugify(&m);
        if !slug.is_empty() {
            out.push(Cohort {
                key: format!("make:{slug}"),
                kind: "make".into(),
                label: format!("{m} ships"),
            });
        }
    }

    for ra in ranges_for(category) {
        if let Some(v) = num_path(metadata, ra.path) {
            if let Some(b) = ra.buckets.iter().find(|b| v >= b.min && v < b.max) {
                out.push(Cohort {
                    key: format!("range:{}:{}", ra.attr, b.key),
                    kind: "range".into(),
                    label: b.label.to_string(),
                });
            }
        }
    }

    out
}

/// Just the keys — used by the stats builder bucketing.
pub fn cohort_keys(category: &str, metadata: &Value) -> Vec<String> {
    cohort_memberships(category, metadata)
        .into_iter()
        .map(|c| c.key)
        .collect()
}

fn str_field(meta: &Value, key: &str) -> Option<String> {
    meta.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn manufacturer(meta: &Value) -> Option<String> {
    match meta.get("manufacturer") {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Some(Value::Object(o)) => o
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        _ => None,
    }
}

fn num_path(meta: &Value, path: &str) -> Option<f64> {
    let mut cur = meta;
    for seg in path.split('.') {
        cur = cur.get(seg)?;
    }
    match cur {
        Value::Number(n) => n.as_f64(),
        _ => None,
    }
}

fn slugify(s: &str) -> String {
    let out: String = s
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    out.trim_matches('-').chars().take(48).collect()
}

fn pluralish(t: &str) -> String {
    if t.ends_with('s') {
        t.to_string()
    } else {
        format!("{t}s")
    }
}

fn family_label(category: &str, fam: &str) -> String {
    if category == "vehicle" {
        match fam {
            "combat" => "Combat ships",
            "industrial" => "Industrial ships",
            "transport" => "Transport ships",
            "support" => "Support ships",
            "ground" => "Ground vehicles",
            _ => "Other vehicles",
        }
        .to_string()
    } else {
        let mut c = fam.chars();
        match c.next() {
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            None => "Other".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn vehicle_memberships_cover_family_type_make_ranges() {
        let meta = json!({ "role": "Interceptor", "manufacturer": "Aegis Dynamics", "cargo_capacity": 0, "crew": { "max": 1 }, "msrp": 60 });
        let keys: Vec<String> = cohort_keys("vehicle", &meta);
        assert!(keys.contains(&"family:combat".to_string()));
        assert!(keys.contains(&"type:interceptor".to_string()));
        assert!(keys.contains(&"make:aegis-dynamics".to_string()));
        assert!(keys.contains(&"range:cargo:0-10".to_string()));
        assert!(keys.contains(&"range:crew:1".to_string()));
        assert!(keys.contains(&"range:price:0-75".to_string()));
    }

    #[test]
    fn range_buckets_are_half_open() {
        let m = json!({ "cargo_capacity": 100 });
        assert!(cohort_keys("vehicle", &m).contains(&"range:cargo:100-1000".to_string()));
        let m2 = json!({ "cargo_capacity": 1500 });
        assert!(cohort_keys("vehicle", &m2).contains(&"range:cargo:1000+".to_string()));
    }

    #[test]
    fn missing_attribute_yields_no_range_cohort() {
        let m = json!({ "role": "Interceptor" });
        let keys = cohort_keys("vehicle", &m);
        assert!(keys.iter().all(|k| !k.starts_with("range:")));
        assert!(keys.iter().all(|k| !k.starts_with("make:")));
        assert!(keys.contains(&"type:interceptor".to_string()));
    }

    #[test]
    fn manufacturer_object_form() {
        let m = json!({ "manufacturer": { "name": "Anvil Aerospace" } });
        assert!(cohort_keys("vehicle", &m).contains(&"make:anvil-aerospace".to_string()));
    }

    #[test]
    fn non_vehicle_uses_type_label_and_no_ranges_when_attrs_absent() {
        // No size/range/mass numeric attrs present → no range cohorts,
        // but the type cohort still resolves from `type_label`.
        let m = json!({ "type_label": "Ballistic Cannon" });
        let keys = cohort_keys("weapon", &m);
        assert!(keys.contains(&"type:ballistic-cannon".to_string()));
        assert!(keys.iter().all(|k| !k.starts_with("range:")));
    }

    #[test]
    fn weapon_item_location_emit_range_cohorts_from_real_paths() {
        let w = json!({ "size": 2, "personal_weapon": { "effective_range": 120 } });
        let wk = cohort_keys("weapon", &w);
        assert!(wk.contains(&"range:size:2".to_string()));
        assert!(wk.contains(&"range:range:60-150".to_string()));

        let i = json!({ "size": 1, "mass": 12.0 });
        let ik = cohort_keys("item", &i);
        assert!(ik.contains(&"range:size:1".to_string()));
        assert!(ik.contains(&"range:mass:5-50".to_string()));

        let l = json!({ "mission_count": 80, "size": 4000.0 });
        let lk = cohort_keys("location", &l);
        assert!(lk.contains(&"range:missions:50-200".to_string()));
        assert!(lk.contains(&"range:diameter:1000-100000".to_string()));
    }
}
