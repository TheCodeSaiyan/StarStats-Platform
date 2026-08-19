//! `cargo run -p starstats-core --example coverage -- <path-to-Game.log>`
//!
//! One-shot parser-coverage report. Reads a Game.log, runs every line
//! through `structural_parse` then `classify`, and prints:
//!
//!   * total lines, recognised count, structural-only count, skipped count
//!   * coverage %  (recognised / (recognised + structural_only))
//!   * top unrecognised event_names with counts and a sample body
//!
//! This mirrors what the tray client now records into SQLite, but as a
//! standalone tool you can point at any Game.log without touching the
//! tray's stored state.
//!
//! No external deps beyond the workspace — keeps it portable.
//!
//! NOTE: cfg!(target_os = "windows") paths use UTF-16; we read as bytes
//! and lossy-decode so a corrupted line never aborts the run.

use starstats_core::{classify, structural_parse};
use std::collections::HashMap;
use std::io::{BufReader, Read};
use std::path::PathBuf;

#[derive(Default)]
struct UnknownAcc {
    count: usize,
    first_line: String,
    first_body: String,
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = match args.next() {
        Some(p) => PathBuf::from(p),
        None => {
            eprintln!("usage: coverage <path-to-Game.log>");
            std::process::exit(2);
        }
    };

    let file = match std::fs::File::open(&path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("open {}: {e}", path.display());
            std::process::exit(1);
        }
    };

    // Read raw bytes so a stray non-UTF8 byte (rare, but the game has
    // emitted UTF-16 BOMs on Linux/Proton) doesn't abort the run.
    let mut buf = Vec::new();
    if let Err(e) = BufReader::new(file).read_to_end(&mut buf) {
        eprintln!("read {}: {e}", path.display());
        std::process::exit(1);
    }
    let text = String::from_utf8_lossy(&buf);

    let mut total = 0u64;
    let mut recognised = 0u64;
    let mut structural_only = 0u64;
    let mut skipped = 0u64;

    let mut unknowns: HashMap<String, UnknownAcc> = HashMap::new();
    let mut recognised_by_type: HashMap<String, u64> = HashMap::new();

    for line in text.lines() {
        total += 1;
        let Some(parsed) = structural_parse(line) else {
            skipped += 1;
            continue;
        };
        if let Some(event) = classify(&parsed) {
            recognised += 1;
            // Pull the discriminant cheaply via Debug — the GameEvent
            // enum's variant name is fine for a count-by-type table.
            let dbg = format!("{event:?}");
            let variant = dbg.split('(').next().unwrap_or("?").to_string();
            *recognised_by_type.entry(variant).or_default() += 1;
        } else {
            structural_only += 1;
            if let Some(name) = parsed.event_name {
                let acc = unknowns.entry(name.to_string()).or_default();
                acc.count += 1;
                if acc.first_line.is_empty() {
                    acc.first_line = line.to_string();
                    acc.first_body = parsed.body.to_string();
                }
            }
        }
    }

    let denom = recognised + structural_only;
    let coverage_pct = if denom == 0 {
        0.0
    } else {
        100.0 * recognised as f64 / denom as f64
    };

    println!("\n=== StarStats parser coverage ===");
    println!("file       : {}", path.display());
    println!("total lines: {total}");
    println!("recognised : {recognised}");
    println!("unknown    : {structural_only}  (parsed shell, no classifier)");
    println!("skipped    : {skipped}  (banners/blanks/no shell)");
    println!("coverage   : {coverage_pct:.1}%  (of recognisable lines)");

    println!("\n--- recognised events by type ---");
    let mut by_type: Vec<_> = recognised_by_type.into_iter().collect();
    by_type.sort_by(|a, b| b.1.cmp(&a.1));
    for (variant, count) in &by_type {
        println!("  {count:>6}  {variant}");
    }

    println!("\n--- top unknown event_names ---");
    let mut top: Vec<_> = unknowns.into_iter().collect();
    top.sort_by(|a, b| b.1.count.cmp(&a.1.count));
    for (name, acc) in top.iter().take(40) {
        println!("  {:>6}  <{}>", acc.count, name);
        // First-occurrence body sample, trimmed to keep output readable.
        let sample = acc.first_body.replace('\t', " ");
        let sample = if sample.len() > 200 {
            format!("{}…", &sample[..200])
        } else {
            sample
        };
        println!("          {sample}");
    }
    if top.len() > 40 {
        println!("  …and {} more event types", top.len() - 40);
    }
}
