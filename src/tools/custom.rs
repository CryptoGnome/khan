use super::{shell, ToolCtx};
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::collections::HashMap;

/// Tool names reserved by the built-in and control tools — custom tools cannot shadow them.
pub const RESERVED: &[&str] = &[
    "read_file", "write_file", "list_files", "shell", "web_fetch", "web_search", "sql",
    "remember", "recall", "credits", "finish", "hire", "delegate", "fire", "list_team",
    "update_prompt", "rollback_prompt", "save_playbook", "create_tool", "rollback_tool",
    "create_skill", "use_skill", "rollback_skill",
];

/// Schemas for the tool-registry management tools (available to every agent).
pub fn management_schemas() -> Vec<Value> {
    vec![
        json!({"type": "function", "function": {"name": "create_tool",
            "description": "Create a NEW custom tool, or improve an existing one (same name = new version, old versions kept). The tool immediately becomes available to ALL agents as a real callable tool. The script reads its arguments as JSON from the KHAN_TOOL_ARGS environment variable and prints its result to stdout. It runs in the workspace directory with a 120s timeout.",
            "parameters": {"type": "object", "properties": {
                "name": {"type": "string", "description": "snake_case tool name"},
                "description": {"type": "string", "description": "What the tool does — shown to agents deciding whether to call it"},
                "parameters": {"type": "object", "description": "JSON Schema for the tool's arguments, e.g. {\"type\":\"object\",\"properties\":{\"text\":{\"type\":\"string\"}},\"required\":[\"text\"]}"},
                "lang": {"type": "string", "enum": ["python", "bash", "powershell"], "description": "python works everywhere; bash on Linux; powershell on Windows (or where pwsh is installed)"},
                "script": {"type": "string", "description": "Full script source. Read args from the KHAN_TOOL_ARGS env var (JSON), print the result to stdout."},
                "reason": {"type": "string", "description": "Why this tool / this improvement"}},
                "required": ["name", "description", "parameters", "lang", "script", "reason"]}}}),
        json!({"type": "function", "function": {"name": "rollback_tool",
            "description": "Revert a custom tool to its previous version (use if an improvement made it worse).",
            "parameters": {"type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"]}}}),
    ]
}

/// Schemas for all registered custom tools (latest versions).
pub fn registry_schemas(ctx: &ToolCtx) -> Vec<Value> {
    ctx.store
        .list_tools()
        .into_iter()
        .map(|(name, desc, params)| {
            let parameters: Value = serde_json::from_str(&params)
                .unwrap_or_else(|_| json!({"type": "object", "properties": {}}));
            json!({"type": "function", "function": {
                "name": name, "description": format!("[custom tool] {desc}"), "parameters": parameters}})
        })
        .collect()
}

pub fn create(ctx: &ToolCtx, args: &Value) -> Result<String> {
    let name = args["name"].as_str().unwrap_or("").trim().to_string();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        bail!("tool name must be non-empty snake_case (ascii letters, digits, underscores)");
    }
    if RESERVED.contains(&name.as_str()) {
        bail!("'{name}' is a built-in tool name and cannot be replaced");
    }
    let lang = args["lang"].as_str().unwrap_or("");
    if !matches!(lang, "python" | "bash" | "powershell") {
        bail!("lang must be 'python', 'bash', or 'powershell'");
    }
    let params = &args["parameters"];
    if !params.is_object() || params["type"] != "object" {
        bail!("parameters must be a JSON Schema object with \"type\": \"object\"");
    }
    let script = args["script"].as_str().unwrap_or("");
    if script.trim().is_empty() {
        bail!("script must not be empty");
    }
    // The launcher command a custom tool runs through never mentions gh — the
    // script body is where a gh call would hide, so it is scanned at create
    // time with the same line-level check the shell uses. Ceiling: this catches
    // command-position invocations (bash/powershell lines, python os.system
    // one-liners), not gh reached through argv-list indirection.
    if shell::touches_gh(script) {
        bail!("script invokes gh, which is not available (it would use the founder's personal GitHub login). Use the gh_api tool for GitHub work.");
    }
    let v = ctx.store.save_tool(
        &name,
        args["description"].as_str().unwrap_or(""),
        &params.to_string(),
        lang,
        script,
        args["reason"].as_str().unwrap_or(""),
    )?;
    Ok(if v == 1 {
        format!("tool '{name}' created and now available to all agents")
    } else {
        format!("tool '{name}' updated to version {v} (rollback_tool reverts)")
    })
}

/// Execute a registered custom tool by name. Returns None if no such tool exists.
pub async fn run(ctx: &ToolCtx, name: &str, args: &Value) -> Option<Result<String>> {
    let (_, _, lang, script) = ctx.store.get_tool(name)?;
    Some(run_inner(ctx, name, &lang, &script, args).await)
}

async fn run_inner(ctx: &ToolCtx, name: &str, lang: &str, script: &str, args: &Value) -> Result<String> {
    let dir = ctx.workspace.join(".tools");
    std::fs::create_dir_all(&dir)?;
    let ext = match lang {
        "python" => "py",
        "bash" => "sh",
        _ => "ps1",
    };
    let path = dir.join(format!("{name}.{ext}"));
    std::fs::write(&path, script)?;
    let py = if cfg!(windows) { "python" } else { "python3" };
    let cmd = match lang {
        "python" => format!("{py} \".tools/{name}.py\""),
        "bash" => format!("sh \".tools/{name}.sh\""),
        // powershell: native syntax on Windows, pwsh (if installed) on Linux
        _ if cfg!(windows) => format!("& \".tools/{name}.ps1\""),
        _ => format!("pwsh -File \".tools/{name}.ps1\""),
    };
    let mut env = HashMap::new();
    env.insert("KHAN_TOOL_ARGS".to_string(), args.to_string());
    shell::run_with_env(ctx, &cmd, None, env).await
}
