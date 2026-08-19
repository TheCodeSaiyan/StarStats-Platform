//! `cargo run -q -p starstats-client --example reset_cursor`
//!
//! One-shot dev helper: rewinds the tail cursor to byte 0 and clears
//! the unknown_event_samples table, so the next tray run reprocesses
//! the entire current Game.log. Idempotency keys protect the events
//! table from double-inserts.

use rusqlite::Connection;

fn main() -> rusqlite::Result<()> {
    let dirs =
        directories::ProjectDirs::from("app", "StarStats", "tray").expect("project dirs resolve");
    let db = dirs.data_dir().join("data.sqlite3");
    println!("db: {}", db.display());

    let conn = Connection::open(&db)?;
    let cursors = conn.execute("UPDATE tail_cursor SET offset = 0", [])?;
    let unknowns = conn.execute("DELETE FROM unknown_event_samples", [])?;
    println!("reset {cursors} cursor row(s); cleared {unknowns} unknown sample(s)");
    Ok(())
}
