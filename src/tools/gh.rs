use super::ToolCtx;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

/// GitHub via the founder-provided account token. The token rides cfg.keys
/// (captured at load, scrubbed from the env) and every call happens in-binary
/// — no git credential ever reaches an agent shell, preserving the "no push
/// path from the container" property the repo audit verified. The REST API
/// covers the whole lane shell-git would: create repos, commit files
/// (contents API), fork, branch, open PRs and issues.
pub fn schemas(ctx: &ToolCtx) -> Vec<Value> {
    // No token, no tool — nothing to reason about until the founder wires it.
    if ctx.cfg.secret("GITHUB_TOKEN").is_none() {
        return vec![];
    }
    vec![json!({"type": "function", "function": {
        "name": "gh_api",
        "description": "Call the GitHub REST API as the company's own GitHub account (founder-provided token, public_repo scope). Covers everything git would: create repos (POST /user/repos), commit files (PUT /repos/{owner}/{repo}/contents/{path} with base64 content), fork (POST /repos/{owner}/{repo}/forks), branches (POST .../git/refs), PRs (POST .../pulls), issues. DISCIPLINE: the company's own repos freely; PRs/issues on other people's repos only per a CEO-approved plan and never spammy; NEVER touch the founder's repos (CryptoGnome/*) — the company's code ships via the founder, not via this token. Responses are truncated to 3000 chars; use per-page params to keep reads small.",
        "parameters": {"type": "object", "properties": {
            "method": {"type": "string", "enum": ["GET", "POST", "PUT", "PATCH", "DELETE"]},
            "path": {"type": "string", "description": "API path starting with /, e.g. /user/repos or /repos/{owner}/{repo}/contents/{path}"},
            "body": {"type": "string", "description": "Optional JSON request body as a string"}},
            "required": ["method", "path"]}}})]
}

pub async fn api(ctx: &ToolCtx, method: &str, path: &str, body: &str) -> Result<String> {
    let token = ctx.cfg.secret("GITHUB_TOKEN").context("GITHUB_TOKEN not configured")?;
    let path = path.trim();
    if !path.starts_with('/') {
        bail!("path must start with /");
    }
    // The founder's own repos are out of bounds in BOTH directions: pushes to
    // CryptoGnome/khan auto-deploy the live company, and this token must never
    // become a path to that.
    if path.to_lowercase().contains("/repos/cryptognome") {
        bail!("the founder's repos are off-limits to this token — company code ships via the founder");
    }
    let url = format!("https://api.github.com{path}");
    let mut req = match method {
        "GET" => ctx.http.get(&url),
        "POST" => ctx.http.post(&url),
        "PUT" => ctx.http.put(&url),
        "PATCH" => ctx.http.patch(&url),
        "DELETE" => ctx.http.delete(&url),
        _ => bail!("method must be GET/POST/PUT/PATCH/DELETE"),
    };
    req = req
        .bearer_auth(token)
        .header("User-Agent", "khan-company")
        .header("Accept", "application/vnd.github+json");
    let body = body.trim();
    if !body.is_empty() {
        let parsed: Value = serde_json::from_str(body).context("body must be valid JSON")?;
        req = req.json(&parsed);
    }
    let resp = req.send().await.context("github request failed")?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let out: String = text.chars().take(3000).collect();
    if !status.is_success() {
        bail!("github returned {status}: {out}");
    }
    Ok(format!("[{status}] {out}"))
}
