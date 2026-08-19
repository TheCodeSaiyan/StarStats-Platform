//! Replay the captured Game.log against the parser and assert
//! coverage characteristics. This is integration-level — it loads
//! the real fixture file from disk.

use starstats_core::{classify, structural_parse, ParseStats};
use std::fs;
use std::path::PathBuf;

fn fixture_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests/fixtures/sample_game_log.txt");
    p
}

fn replay() -> ParseStats {
    let path = fixture_path();
    let content = fs::read_to_string(&path).expect("fixture readable");
    let mut stats = ParseStats::default();
    for line in content.lines() {
        if line.is_empty() {
            continue;
        }
        if let Some(parsed) = structural_parse(line) {
            let classified = classify(&parsed);
            stats.record(classified.is_some(), true);
        } else {
            stats.record(false, false);
        }
    }
    stats
}

#[test]
fn fixture_structural_coverage_is_high() {
    // The fixture is a real PUB session capture (sc-alpha-4.7.0).
    // Most lines should structurally parse; raw banner lines won't.
    let stats = replay();
    assert!(
        stats.total >= 1500,
        "fixture should have ≥1500 lines, got {}",
        stats.total
    );
    let structural_or_recognised = stats.recognised + stats.structural_only;
    let ratio = structural_or_recognised as f64 / stats.total as f64;
    assert!(
        ratio >= 0.90,
        "expected ≥90% structural parse rate, got {:.1}% ({}/{})",
        ratio * 100.0,
        structural_or_recognised,
        stats.total
    );
}

#[test]
fn fixture_recognises_session_anchors() {
    // We expect to recognise the Init / Login / Join PU events at
    // minimum from this fixture, even without combat data.
    let path = fixture_path();
    let content = fs::read_to_string(&path).expect("fixture readable");
    let mut found_init = false;
    let mut found_login = false;
    let mut found_join_pu = false;
    let mut found_change_server = false;

    for line in content.lines() {
        let Some(parsed) = structural_parse(line) else {
            continue;
        };
        let Some(event) = classify(&parsed) else {
            continue;
        };
        match event {
            starstats_core::GameEvent::ProcessInit(_) => found_init = true,
            starstats_core::GameEvent::LegacyLogin(_) => found_login = true,
            starstats_core::GameEvent::JoinPu(_) => found_join_pu = true,
            starstats_core::GameEvent::ChangeServer(_) => found_change_server = true,
            _ => {}
        }
    }

    assert!(found_init, "expected ProcessInit");
    assert!(found_login, "expected LegacyLogin");
    assert!(found_join_pu, "expected JoinPu");
    assert!(found_change_server, "expected ChangeServer");
}
