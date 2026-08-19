//! `cargo run -q -p starstats-client --example inspect`
//!
//! One-shot query of the local SQLite store: cursors, event counts,
//! a few recent events. Useful when there's no `sqlite3` CLI on the
//! box and we just want to sanity-check that the tailer is writing.

use rusqlite::Connection;

fn main() -> rusqlite::Result<()> {
    let dirs =
        directories::ProjectDirs::from("app", "StarStats", "tray").expect("project dirs resolve");
    let db_path = dirs.data_dir().join("data.sqlite3");
    println!("db: {}", db_path.display());

    let conn = Connection::open(&db_path)?;

    println!("\n--- tail_cursor ---");
    let mut stmt = conn.prepare("SELECT path, offset FROM tail_cursor")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
    for row in rows {
        let (p, b) = row?;
        println!("  {b:>10}  {p}");
    }

    println!("\n--- event counts by type ---");
    let mut stmt =
        conn.prepare("SELECT type, COUNT(*) FROM events GROUP BY type ORDER BY 2 DESC")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
    let mut total = 0i64;
    for row in rows {
        let (t, c) = row?;
        total += c;
        println!("  {c:>6}  {t}");
    }
    println!("  ------");
    println!("  {total:>6}  TOTAL");

    println!("\n--- 10 most recent events ---");
    let mut stmt =
        conn.prepare("SELECT timestamp, type, payload FROM events ORDER BY id DESC LIMIT 10")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    for row in rows {
        let (ts, et, payload) = row?;
        let preview = if payload.len() > 100 {
            format!("{}…", &payload[..100])
        } else {
            payload
        };
        println!("  {ts}  [{et}]  {preview}");
    }

    println!("\n--- top 10 unknown event names ---");
    let mut stmt = conn.prepare(
        "SELECT event_name, occurrences FROM unknown_event_samples ORDER BY occurrences DESC LIMIT 10",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
    for row in rows {
        let (n, c) = row?;
        println!("  {c:>6}  <{n}>");
    }

    println!("\n--- noise list ---");
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM event_noise_list", [], |r| r.get(0))?;
    println!("  {n} entries");
    let mut stmt = conn
        .prepare("SELECT event_name, source FROM event_noise_list ORDER BY source, event_name")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    for row in rows.take(8) {
        let (n, s) = row?;
        println!("  [{s}] {n}");
    }
    if n > 8 {
        println!("  ... and {} more", n - 8);
    }

    Ok(())
}
