pub mod credits;
pub mod custom;
pub mod gh;
pub mod x;
pub mod skills;
pub(crate) mod fs;
mod image;
pub mod shell;
pub mod sql;
mod web;

use crate::config::Config;
use crate::state::Store;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone)]
pub struct ToolCtx {
    pub cfg: Config,
    pub store: Arc<Store>,
    pub workspace: PathBuf,
    pub http: reqwest::Client,
    /// Client routed through FETCH_PROXY (residential proxy), when configured.
    /// Only web fetch/search fall back to it — RPC and model API traffic must
    /// never transit a third-party proxy.
    pub http_proxy: Option<reqwest::Client>,
}

/// Public schema builder for callers outside this module (agent.rs adds
/// conditional CEO tools like message_founder).
pub fn tool_schema(name: &str, desc: &str, params: Value) -> Value {
    tool(name, desc, params)
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
        tool("write_file", "Write a text file in the workspace. Overwrites by default.", json!({
            "properties": {"path": {"type": "string"}, "content": {"type": "string"},
                           "append": {"type": "boolean", "description": "Add to the end of the file instead of overwriting it. This is how to build a file too large to fit in one response: write the first chunk, then append each following chunk."}},
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
        tool("generate_image", "Generate a real image (coin art, site imagery, social graphics) and save it as a PNG in the workspace. Runs on OpenRouter image models — a few tenths of a cent per image at the default; NEVER hand-draw art with PIL when this tool exists. PROMPTING: write ONE flowing sentence, not a keyword pile, front-loading the subject — [subject with key details] → [style/medium] → [composition/shot] → [lighting/color]. Put any words that must appear IN the image in \"double quotes\". Phrase avoids as positives ('clean empty background', never 'no clutter' — negatives are ignored). Iterate by changing ONE thing, not re-rolling. Look at the saved file (or its byte size — under ~30KB usually means a failed/blank render) before shipping it anywhere.", json!({
            "properties": {"prompt": {"type": "string", "description": "The full one-sentence image description."},
                           "path": {"type": "string", "description": "Workspace-relative output path ending in .png"},
                           "model": {"type": "string", "description": "Optional OpenRouter image model override for when the default's render disappoints: 'x-ai/grok-imagine-image-2.0' ($0.06, strong photoreal) or 'qwen/qwen-image-3-pro' ($0.075, high detail). Leave empty for the $0.01 default."}},
            "required": ["prompt", "path"]})),
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

/// Stamp the live workspace.db table list into the sql tool's description.
/// The schema-on-error reply teaches columns after a miss, but agents still
/// paid a model iteration per guessed name (10 misses in 90 minutes,
/// 2026-08-31) — the table list belongs where the query gets written.
/// Schemas rebuild every iteration, so new tables appear at once.
pub fn hint_sql_tables(workspace: &std::path::Path, schemas: &mut [Value]) {
    let Some(names) = sql::table_names(workspace) else { return };
    for t in schemas.iter_mut() {
        if t["function"]["name"] == "sql" {
            if let Some(d) = t["function"]["description"].as_str() {
                t["function"]["description"] = format!("{d} Existing tables: {names}. A name not in this list does not exist — check the list before inventing one.").into();
            }
        }
    }
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

/// How long a spill file stays useful: an agent reads the tail back within the
/// episode that produced it, so a week is generous.
const SPILL_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 3600);

/// Delete spill files older than `max_age`. Best-effort: an unreadable entry
/// or failed remove is skipped, never an error — cleanup must not break the
/// tool result it rides on.
pub(crate) fn purge_spill(dir: &std::path::Path, max_age: std::time::Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > max_age);
        if old {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Over-limit tool output is relocated, not dropped: the full text lands in
/// workspace/.spill/ and the marker names the file, so an agent recovers the
/// tail with read_file instead of re-running an expensive command. Each new
/// spill sweeps week-old ones, so the directory cleans itself on the same
/// path that fills it — no cadence to schedule, nothing to clean in a
/// workspace that never spills.
pub fn truncate_spill(workspace: &std::path::Path, tool: &str, s: String) -> String {
    if s.len() <= MAX_RESULT {
        return s;
    }
    purge_spill(&workspace.join(".spill"), SPILL_MAX_AGE);
    let file = format!("{tool}-{}.txt", chrono::Utc::now().timestamp_micros());
    let saved = std::fs::create_dir_all(workspace.join(".spill"))
        .and_then(|()| std::fs::write(workspace.join(".spill").join(&file), &s))
        .is_ok();
    // Which slice stays visible depends on the tool. Shell-like output (shell,
    // sql, custom tool scripts) buries its error under the preamble, so the
    // TAIL is what matters. Document-like tools lead with what matters — and
    // web content leads with the BEGIN-UNTRUSTED marker, which must never be
    // cut away from an oversized page.
    let keep_tail = !matches!(tool, "web_fetch" | "web_search" | "read_file" | "list_files" | "x_read" | "gh_api" | "recall");
    if keep_tail {
        let mut start = s.len() - MAX_RESULT;
        while !s.is_char_boundary(start) {
            start += 1;
        }
        let marker = if saved {
            format!("[truncated — showing the end; full output saved to .spill/{file}, read_file it if you need the start]\n...")
        } else {
            "[truncated — showing the end]\n...".to_string()
        };
        format!("{marker}{}", &s[start..])
    } else {
        let mut cut = MAX_RESULT;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        let marker = if saved {
            format!("\n...[truncated — full output saved to .spill/{file}; read_file it if you need the rest]")
        } else {
            "\n...[truncated]".to_string()
        };
        format!("{}{marker}", &s[..cut])
    }
}

/// Execute a work tool. Never returns Err — errors become the tool result string.
pub async fn execute(ctx: &ToolCtx, agent: &str, name: &str, args: &Value) -> String {
    // The CEO directs; it does not build. These are stripped from its schema
    // list, but stale history can still produce a call — refuse it with a
    // redirect instead of doing the work.
    if agent == "CEO" && matches!(name, "write_file" | "create_tool") {
        return format!(
            "REFUSED: the CEO does not have {name}. Writing files and building tools is \
             execution — dispatch it to an employee (builder, or hire someone) with clear \
             instructions and rate the result."
        );
    }
    let out = match name {
        "read_file" => fs::read_file(ctx, s(args, "path")),
        "write_file" => fs::write_file(
            ctx,
            s(args, "path"),
            s(args, "content"),
            args["append"].as_bool().unwrap_or(false),
        ),
        "list_files" => fs::list_files(ctx, s(args, "path")),
        "shell" => shell::run(ctx, s(args, "command"), None).await,
        "web_fetch" => web::fetch(ctx, s(args, "url")).await,
        "web_search" => web::search(ctx, s(args, "query")).await,
        "sql" => sql::run(ctx, s(args, "query")),
        "generate_image" => image::generate(ctx, s(args, "prompt"), s(args, "path"), s(args, "model")).await,
        "remember" => {
            ctx.store.remember(agent, s(args, "key"), s(args, "content"), s(args, "tags"));
            Ok("remembered".to_string())
        }
        "recall" => {
            let hits = ctx.store.recall(s(args, "query"), 8);
            Ok(if hits.is_empty() { "no memories found".into() } else { hits.join("\n---\n") })
        }
        "credits" => credits::run(ctx).await,
        "x_post" => x::post(ctx, s(args, "text"), s(args, "reply_to")).await,
        "x_read" => x::read(ctx, s(args, "mode"), s(args, "query")).await,
        "x_topup" => x::topup(ctx, s(args, "tx_signature")).await,
        "gh_api" => gh::api(ctx, s(args, "method"), s(args, "path"), s(args, "body"), s(args, "body_file")).await,
        "create_tool" => custom::create(ctx, args),
        "create_skill" => skills::create(ctx, args),
        "use_skill" => Ok(skills::load(ctx, agent, s(args, "name"))),
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
        // Capped: `text` here is tool OUTPUT — for shell that is the child's whole
        // stdout+stderr, and any command whose output merely starts with "ERROR"
        // lands here — while the activity log is public. A short head is enough to
        // see what broke; the agent still gets the full text as the tool result.
        let brief: String = text.chars().take(400).collect();
        ctx.store.log(agent, &format!("{name}-error"), &brief);
    }
    truncate_spill(&ctx.workspace, name, text)
}
