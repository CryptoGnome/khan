use super::ToolCtx;
use anyhow::{Context, Result};
use rusqlite::Connection;

/// Run SQL against the agents' scratch database (workspace/workspace.db),
/// separate from khan's internal state db.
pub fn run(ctx: &ToolCtx, query: &str) -> Result<String> {
    let path = ctx.workspace.join("workspace.db");
    let conn = Connection::open(&path).context("cannot open workspace.db")?;
    // A wrong guess at a name gets the real schema in the same reply —
    // agents were burning a model iteration per guessed column (mint/symbol/
    // title against positions' actual asset/note, 2026-08-31), and an error
    // that teaches beats a skill nobody re-reads.
    match exec(&conn, query.trim()) {
        // "no such column" is SELECT's wording; an INSERT against a bad column
        // says "has no column named" and slipped past the hint (deliverables_log
        // note, 2026-09-01). Same mistake, same teaching.
        Err(e) if { let m = e.to_string(); m.contains("no such") || m.contains("has no column named") } => {
            Err(e.context(format!("actual schema:\n{}", schema_hint(&conn, query))))
        }
        other => other,
    }
}

/// The scratch DB's table names, comma-joined — None when the db doesn't
/// exist yet or holds nothing. Read-only open so a missing file is never
/// created as a side effect of building tool schemas.
pub fn table_names(workspace: &std::path::Path) -> Option<String> {
    let conn = Connection::open_with_flags(
        workspace.join("workspace.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .ok()?;
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .ok()?;
    let names: Vec<String> =
        stmt.query_map([], |r| r.get::<_, String>(0)).ok()?.flatten().collect();
    if names.is_empty() {
        None
    } else {
        Some(names.join(", "))
    }
}

/// Compact schema summary for error replies, scoped to the tables the failing
/// query actually names.
///
/// Dumping every table defeated the point: workspace.db carries 13 near-identical
/// graduation_watch_* clones that sort before `positions`, so an agent that
/// guessed a column on positions got a wall of unrelated schemas, never reached
/// the line it needed, and guessed again — 132 identical failures in two hours,
/// all of them this error. When no named table matches, the reply falls back to
/// the table NAMES alone, which is the actual answer to "no such table".
fn schema_hint(conn: &Connection, query: &str) -> String {
    let names = table_list(conn);
    if names.is_empty() {
        return "(no tables)".into();
    }
    let words: std::collections::HashSet<String> = query
        .to_lowercase()
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .map(|w| w.to_string())
        .collect();
    let named: Vec<&String> = names.iter().filter(|n| words.contains(&n.to_lowercase())).collect();
    if named.is_empty() {
        return format!("tables: {}", names.join(", "));
    }
    named
        .iter()
        .map(|name| format!("{name}({})", columns_of(conn, name).join(", ")))
        .collect::<Vec<_>>()
        .join("\n")
}

fn table_list(conn: &Connection) -> Vec<String> {
    conn.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .ok()
        .and_then(|mut s| s.query_map([], |r| r.get::<_, String>(0)).ok().map(|it| it.flatten().collect()))
        .unwrap_or_default()
}

fn columns_of(conn: &Connection, table: &str) -> Vec<String> {
    conn.prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
        .ok()
        .and_then(|mut s| s.query_map([], |r| r.get::<_, String>(0)).ok().map(|it| it.flatten().collect()))
        .unwrap_or_default()
}

fn exec(conn: &Connection, q: &str) -> Result<String> {
    let head = q.split_whitespace().next().unwrap_or("").to_lowercase();
    if matches!(head.as_str(), "select" | "with" | "pragma" | "explain") {
        let mut stmt = conn.prepare(q)?;
        let cols: Vec<String> = stmt.column_names().iter().map(|c| c.to_string()).collect();
        let ncols = cols.len();
        let mut rows_out = vec![cols.join(" | ")];
        let mut rows = stmt.query([])?;
        let mut n = 0;
        while let Some(row) = rows.next()? {
            let mut vals = Vec::with_capacity(ncols);
            for i in 0..ncols {
                let v: rusqlite::types::Value = row.get(i)?;
                vals.push(match v {
                    rusqlite::types::Value::Null => "NULL".into(),
                    rusqlite::types::Value::Integer(i) => i.to_string(),
                    rusqlite::types::Value::Real(f) => f.to_string(),
                    rusqlite::types::Value::Text(t) => t,
                    rusqlite::types::Value::Blob(b) => format!("<blob {} bytes>", b.len()),
                });
            }
            rows_out.push(vals.join(" | "));
            n += 1;
            if n >= 200 {
                rows_out.push("...[more rows omitted]".into());
                break;
            }
        }
        Ok(rows_out.join("\n"))
    } else {
        // Allow multi-statement DDL/DML batches.
        conn.execute_batch(q)?;
        Ok("OK".into())
    }
}
