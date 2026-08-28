pub mod credits;
pub mod custom;
pub mod skills;
mod fs;
pub mod shell;
mod sql;
mod web;

use crate::config::Config;
use crate::state::Store;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;

pub struct ToolCtx {
    pub cfg: Config,
    pub store: Arc<Store>,
    pub workspace: PathBuf,
    pub http: reqwest::Client,
}

fn tool(name: &str, desc: &str, params: Value) -> Value {
    json!({"type": "function", "function": {"name": name, "description": desc,
        "parameters": {"type": "object", "properties": params["properties"],
                       "required": params["required"]}}})
}

/// Schemas for the work tools every agent gets. CEO-only control tools are added in agent.rs.
pub fn work_schemas() -> Vec<Value> {
    vec![
        tool("read_file", "Read a text file from the workspace.", json!({
            "properties": {"path": {"type": "string", "description": "Path relative to the workspace"}},
            "required": ["path"]})),
        tool("write_file", "Write (create/overwrite) a text file in the workspace.", json!({
            "properties": {"path": {"type": "string"}, "content": {"type": "string"}},
            "required": ["path", "content"]})),
        tool("list_files", "List files under a workspace directory (recursive).", json!({
            "properties": {"path": {"type": "string", "description": "Relative dir, '' for workspace root"}},
            "required": []})),
        tool("shell", &format!("Run a command in the workspace directory using the system shell ({}). 120s timeout.", shell::SHELL_NAME), json!({
            "properties": {"command": {"type": "string"},
                           "purpose": {"type": "string", "description": "REQUIRED. One short plain-English sentence saying what this command is for, written for a non-technical person watching the public activity log — e.g. 'checking the treasury balance on-chain' or 'installing the Solana python library'. Never restate the code; say the goal."}},
            "required": ["command", "purpose"]})),
        tool("web_fetch", "Fetch a URL and return its text content.", json!({
            "properties": {"url": {"type": "string"}},
            "required": ["url"]})),
        tool("web_search", "Search the web (DuckDuckGo). Returns result titles, URLs and snippets.", json!({
            "properties": {"query": {"type": "string"}},
            "required": ["query"]})),
        tool("sql", "Run SQL against the company scratch database workspace.db (SQLite). Use it for any structured data you want to keep and query.", json!({
            "properties": {"query": {"type": "string"},
                           "purpose": {"type": "string", "description": "REQUIRED. One short plain-English sentence saying what this query is for, written for a non-technical person watching the public activity log — e.g. 'recording today's profit in the ledger'. Never restate the SQL; say the goal."}},
            "required": ["query", "purpose"]})),
        tool("remember", "Store a memory (fact, decision, lesson) in long-term memory.", json!({
            "properties": {"key": {"type": "string", "description": "Short title"},
                           "content": {"type": "string"},
                           "tags": {"type": "string", "description": "Comma-separated tags"}},
            "required": ["key", "content"]})),
        tool("recall", "Full-text search long-term memory.", json!({
            "properties": {"query": {"type": "string"}},
            "required": ["query"]})),
    ]
}

fn s<'a>(args: &'a Value, k: &str) -> &'a str {
    args[k].as_str().unwrap_or("")
}

pub const MAX_RESULT: usize = 12_000;

pub fn truncate(mut s: String) -> String {
    if s.len() > MAX_RESULT {
        let mut cut = MAX_RESULT;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
        s.push_str("\n...[truncated]");
    }
    s
}

/// Execute a work tool. Never returns Err — errors become the tool result string.
pub async fn execute(ctx: &ToolCtx, agent: &str, name: &str, args: &Value) -> String {
    let out = match name {
        "read_file" => fs::read_file(ctx, s(args, "path")),
        "write_file" => fs::write_file(ctx, s(args, "path"), s(args, "content")),
        "list_files" => fs::list_files(ctx, s(args, "path")),
        "shell" => shell::run(ctx, s(args, "command"), None).await,
        "web_fetch" => web::fetch(ctx, s(args, "url")).await,
        "web_search" => web::search(ctx, s(args, "query")).await,
        "sql" => sql::run(ctx, s(args, "query")),
        "remember" => {
            ctx.store.remember(agent, s(args, "key"), s(args, "content"), s(args, "tags"));
            Ok("remembered".to_string())
        }
        "recall" => {
            let hits = ctx.store.recall(s(args, "query"), 8);
            Ok(if hits.is_empty() { "no memories found".into() } else { hits.join("\n---\n") })
        }
        "credits" => credits::run(ctx).await,
        "create_tool" => custom::create(ctx, args),
        "create_skill" => skills::create(ctx, args),
        "use_skill" => Ok(skills::load(ctx, s(args, "name"))),
        "rollback_skill" => match ctx.store.rollback_skill(s(args, "name")) {
            Ok(true) => Ok("rolled back to previous version".into()),
            Ok(false) => Ok("nothing to roll back".into()),
            Err(e) => Ok(format!("ERROR: {e:#}")),
        },
        "rollback_tool" => match ctx.store.rollback_tool(s(args, "name")) {
            Ok(true) => Ok("rolled back to previous version".into()),
            Ok(false) => Ok("nothing to roll back".into()),
            Err(e) => Ok(format!("ERROR: {e:#}")),
        },
        _ => match custom::run(ctx, name, args).await {
            Some(r) => r,
            None => Ok(format!("unknown tool: {name}")),
        },
    };
    let text = out.unwrap_or_else(|e| format!("ERROR: {e:#}"));
    // Record every outcome so a tool that is broken in this environment shows up
    // as a pattern at reflection instead of being silently re-tried forever.
    let failed = text.starts_with("ERROR");
    ctx.store.record_tool_call(name, !failed, if failed { &text } else { "" });
    if failed {
        // Surface it in the activity log too (the viewer styles *-error red).
        ctx.store.log(agent, &format!("{name}-error"), &text);
    }
    truncate(text)
}
