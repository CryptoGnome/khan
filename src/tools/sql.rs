use super::ToolCtx;
use anyhow::{Context, Result};
use rusqlite::Connection;

/// Run SQL against the agents' scratch database (workspace/workspace.db),
/// separate from khan's internal state db.
pub fn run(ctx: &ToolCtx, query: &str) -> Result<String> {
    let path = ctx.workspace.join("workspace.db");
    let conn = Connection::open(&path).context("cannot open workspace.db")?;
    let q = query.trim();
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
