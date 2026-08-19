//! Peer-group classifier — buckets a reference entity into a "class"
//! so stats can be computed relative to comparable peers. Shared by the
//! server stats builder and the detail handler so both agree on a row's
//! group (no client/server drift). Returns lowercase, bounded strings
//! that keep ASCII alphanumerics plus underscores and map every other
//! character to `-`; unknown inputs fall back to `"other"`.

use serde_json::Value;

/// Generous ceiling; keeps the bucket key bounded without truncating real taxonomy values.
const MAX_SLUG_LEN: usize = 48;

/// Bucket `metadata` into a peer group for `category`. Lowercase,
/// hyphenated, bounded set. Unknown → `"other"`.
pub fn peer_group(category: &str, metadata: &Value) -> String {
    match category {
        "vehicle" => vehicle_family(str_field(metadata, "role")),
        "weapon" | "item" => slugify_label(str_field(metadata, "type_label")),
        "location" => slugify_label(str_field(metadata, "tier")),
        _ => "other".to_string(),
    }
}

fn str_field<'a>(meta: &'a Value, key: &str) -> &'a str {
    meta.get(key).and_then(Value::as_str).unwrap_or("")
}

/// Coarse vehicle role-family from the free-form role string. Keyword
/// match is case-insensitive and order-sensitive (first hit wins).
fn vehicle_family(role: &str) -> String {
    let r = role.to_ascii_lowercase();
    let has = |kw: &str| r.contains(kw);
    let fam = if has("fighter") || has("interceptor") || has("bomber") || has("gunship") {
        "combat"
    } else if has("mining") || has("salvage") || has("refuel") || has("repair") || has("industrial")
    {
        "industrial"
    } else if has("cargo")
        || has("freight")
        || has("transport")
        || has("expedition")
        || has("dropship")
    {
        "transport"
    } else if has("medical") || has("data") || has("racing") || has("pathfinder") || has("science")
    {
        "support"
    } else if has("ground") {
        "ground"
    } else {
        "other"
    };
    fam.to_string()
}

/// Slugify a free-form label; empty → `"other"`. Keeps ASCII
/// alphanumerics and underscores; maps every other character to `-`.
/// Caps length so a malformed wiki field can't produce an unbounded key.
fn slugify_label(label: &str) -> String {
    let s: String = label
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
    let s = s.trim_matches('-').to_string();
    if s.is_empty() {
        "other".to_string()
    } else {
        s.chars().take(MAX_SLUG_LEN).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn vehicle_role_families() {
        let f = |role: &str| peer_group("vehicle", &json!({ "role": role }));
        assert_eq!(f("Interceptor"), "combat");
        assert_eq!(f("Light Fighter"), "combat");
        assert_eq!(f("Stealth Bomber"), "combat");
        assert_eq!(f("Heavy Gunship"), "combat");
        assert_eq!(f("Medical"), "support");
        assert_eq!(f("Racing"), "support");
        assert_eq!(f("Heavy Salvage"), "industrial");
        assert_eq!(f("Heavy Cargo"), "transport");
        assert_eq!(f("Light Freight"), "transport");
        assert_eq!(f("Expedition"), "transport");
        assert_eq!(f("Ground Vehicle"), "ground");
        assert_eq!(f("Mystery Role 9000"), "other");
    }

    #[test]
    fn weapon_and_item_use_type_label() {
        assert_eq!(
            peer_group("weapon", &json!({ "type_label": "Ballistic Cannon" })),
            "ballistic-cannon"
        );
        assert_eq!(
            peer_group("item", &json!({ "type_label": "Cooler" })),
            "cooler"
        );
        assert_eq!(peer_group("weapon", &json!({})), "other");
    }

    #[test]
    fn location_uses_tier() {
        assert_eq!(
            peer_group("location", &json!({ "tier": "landing_zone" })),
            "landing_zone"
        );
        assert_eq!(peer_group("location", &json!({})), "other");
    }

    #[test]
    fn unknown_category_is_other() {
        assert_eq!(peer_group("npc", &json!({ "role": "x" })), "other");
    }
}
