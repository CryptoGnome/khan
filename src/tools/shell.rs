use super::ToolCtx;
use anyhow::Result;
use std::time::Duration;
use tokio::process::Command;

/// Env vars that must never leak into agent-spawned processes (API keys, tokens, …).
pub fn is_sensitive_env(name: &str) -> bool {
    let n = name.to_uppercase();
    ["API_KEY", "APIKEY", "TOKEN", "SECRET", "PASSWORD", "CREDENTIAL"]
        .iter()
        .any(|p| n.contains(p))
}

/// Human name of the shell agents get, by platform (used in tool descriptions/prompts).
pub const SHELL_NAME: &str = if cfg!(windows) { "PowerShell" } else { "POSIX shell (bash/sh)" };

/// Program + fixed args to run one command string, by platform.
#[cfg(windows)]
fn interpreter(command: &str) -> (&'static str, Vec<String>) {
    ("powershell", vec!["-NoProfile".into(), "-NonInteractive".into(), "-Command".into(), command.into()])
}
#[cfg(unix)]
fn interpreter(command: &str) -> (&'static str, Vec<String>) {
    ("sh", vec!["-c".into(), command.into()])
}

/// Run a shell command in the workspace with a 120s timeout.
/// `extra_env` lets custom tools pass their JSON args via KHAN_TOOL_ARGS.
pub async fn run_with_env(
    ctx: &ToolCtx,
    command: &str,
    cwd: Option<&str>,
    extra_env: std::collections::HashMap<String, String>,
) -> Result<String> {
    let dir = match cwd {
        Some(c) if !c.is_empty() => ctx.workspace.join(c),
        _ => ctx.workspace.clone(),
    };
    let (prog, prog_args) = interpreter(command);
    let mut cmd = Command::new(prog);
    cmd.args(&prog_args).current_dir(&dir);
    // Strip secrets from the child env so no command — however an agent was talked
    // into running it — can read or exfiltrate the founder's keys.
    for (name, _) in std::env::vars() {
        if is_sensitive_env(&name) {
            cmd.env_remove(&name);
        }
    }
    cmd.envs(extra_env)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);

    let fut = async {
        let out = cmd.output().await?;
        let mut text = String::from_utf8_lossy(&out.stdout).to_string();
        let err = String::from_utf8_lossy(&out.stderr);
        if !err.trim().is_empty() {
            text.push_str("\n[stderr]\n");
            text.push_str(&err);
        }
        if text.trim().is_empty() {
            text = format!("(no output, exit code {})", out.status.code().unwrap_or(-1));
        } else if !out.status.success() {
            text.push_str(&format!("\n[exit code {}]", out.status.code().unwrap_or(-1)));
        }
        Ok::<String, anyhow::Error>(text)
    };
    match tokio::time::timeout(Duration::from_secs(120), fut).await {
        Ok(r) => r,
        Err(_) => Ok("ERROR: command timed out after 120s and was killed".into()),
    }
}

/// True if a command line invokes gh/hub anywhere in it. Plain git is fine
/// (local version control), but the GitHub CLIs would reach the founder's
/// ambient GitHub login, which agents must never touch.
pub fn touches_gh(command: &str) -> bool {
    // Check only command-position tokens (start of the line or after a separator),
    // so quoted prose mentioning "gh" isn't blocked but `x; gh ...` is.
    command
        .split(|c: char| matches!(c, ';' | '|' | '&' | '(' | ')' | '\n'))
        .map(str::trim)
        .filter_map(|seg| seg.split_whitespace().next())
        .map(|t| {
            let t = t.trim_matches(|c| matches!(c, '"' | '\'')).to_lowercase();
            let base = t.rsplit(|c| matches!(c, '/' | '\\')).next().unwrap_or(&t).to_string();
            base.trim_end_matches(".exe").to_string()
        })
        .any(|t| matches!(t.as_str(), "gh" | "hub"))
}

pub async fn run(ctx: &ToolCtx, command: &str, cwd: Option<&str>) -> Result<String> {
    if touches_gh(command) {
        return Ok("ERROR: gh is not available (it would use the founder's personal GitHub login). Plain git works for local version control in the workspace.".into());
    }
    run_with_env(ctx, command, cwd, std::collections::HashMap::new()).await
}
