use chrono::Timelike;
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

/// One command's outcome, each fact reported on its own: a timeout is not the
/// same failure as a nonzero exit, and folding one into the other forces
/// callers to parse prose (routines want to alert differently on hangs).
pub struct ShellOutcome {
    pub text: String,
    pub success: bool,
    pub timed_out: bool,
}

/// Run one command in `dir` with the scrubbed env and 120s timeout — the same
/// execution agents get from the shell tool. Routines key alerts off the
/// outcome flags.
pub async fn run_in_dir(
    dir: &std::path::Path,
    command: &str,
    extra_env: std::collections::HashMap<String, String>,
) -> Result<ShellOutcome> {
    let (prog, prog_args) = interpreter(command);
    let mut cmd = Command::new(prog);
    cmd.args(&prog_args).current_dir(dir);
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
        Ok::<ShellOutcome, anyhow::Error>(ShellOutcome {
            text,
            success: out.status.success(),
            timed_out: false,
        })
    };
    match tokio::time::timeout(Duration::from_secs(120), fut).await {
        Ok(r) => r,
        Err(_) => Ok(ShellOutcome {
            text: "ERROR: command timed out after 120s and was killed".into(),
            success: false,
            timed_out: true,
        }),
    }
}

/// Run a shell command in the workspace with a 120s timeout.
/// `extra_env` lets custom tools pass their JSON args via KHAN_TOOL_ARGS.
pub async fn run_with_env(
    ctx: &ToolCtx,
    command: &str,
    cwd: Option<&str>,
    extra_env: std::collections::HashMap<String, String>,
) -> Result<String> {
    // The gh guard lives here, not in run(), so every agent-driven execution
    // path — shell tool and custom-tool launcher alike — routes through it.
    if touches_gh(command) {
        return Ok(GH_BLOCKED.into());
    }
    if let Some(word) = pitches_by_mail(command) {
        return Ok(format!("{PITCH_BLOCKED} (matched \"{word}\")"));
    }
    if let Some(word) = moves_live_db(command) {
        return Ok(format!("{DB_BLOCKED} (matched \"{word}\")"));
    }
    if let Some(word) = fires_launch(command) {
        let hour = chrono::Utc::now().hour();
        if !ctx.cfg.launch_window_open(hour) {
            return Ok(format!(
                "{LAUNCH_WINDOW_BLOCKED} The window is {:02}:00–{:02}:00 UTC and it is {hour:02}:xx now (matched \"{word}\").",
                ctx.cfg.launch_window_open_utc, ctx.cfg.launch_window_close_utc
            ));
        }
    }
    let dir = match cwd {
        Some(c) if !c.is_empty() => ctx.workspace.join(c),
        _ => ctx.workspace.clone(),
    };
    run_in_dir(&dir, command, extra_env).await.map(|o| o.text)
}

pub const GH_BLOCKED: &str = "ERROR: gh is not available (it would use the founder's personal GitHub login). Plain git works for local version control in the workspace.";

pub const PITCH_BLOCKED: &str = "REFUSED: this is an outbound email pitching for attention, which the founder banned on 2026-09-02 (directive #155, skill email_policy). The company inbox is transactional only: listing submissions and their replies, account verification, deposit and payment problems, anything a counterparty needs to complete a transaction. Podcasts, interviews, features, partnerships, AMAs, newsletter mentions, press and Show-HN asks are not sends — attention comes from being a good account and shipping things worth talking about. If this send genuinely unblocks money, say which objective and which amount in the command itself and it will pass.";

pub const LAUNCH_WINDOW_BLOCKED: &str = "REFUSED: this is a live token launch and the launch window is closed. Of the eight launches on 2026-09-04/05, five fired between 01:23Z and 08:15Z — the US audience asleep, a dev buy nobody could buy into, and every one of them opened below its floors. Stage the kit (metadata pinned, args file written, dry-run green, the X post drafted) and fire it when the window opens; a staged kit fires in one command, a night launch is a write-off.";

pub const DB_BLOCKED: &str = "REFUSED: khan.db is the running company's open database, and this command would move, replace, or delete the file under it. On 2026-09-03 exactly that (VACUUM INTO a copy, rename the original aside, move the copy into place) left the binary writing to a file nobody could see for seven hours: the board and roster vanished, the site froze, and the X refresh token rotated into a file that was then thrown away. Read it with khan_db_query or sqlite in mode=ro; never rename, copy, truncate, or remove it. Compaction and backups are the founder's, not the company's — if the volume is full, delete evidence dumps and spill files instead.";

/// The marker by which a command fires a LIVE token launch, if it does.
///
/// Every launch since 2026-09-02 ran `launch_experiment.py --live` with
/// `KHAN_ALLOW_LAUNCH=yes`; a dry-run carries neither. Ceiling: a launcher
/// under another name with its own gate slips past — the brief's window line
/// is the second fence, and the launch record in workspace.db the audit.
pub fn fires_launch(command: &str) -> Option<&'static str> {
    let c = command.to_lowercase();
    if c.contains("khan_allow_launch=yes") {
        return Some("KHAN_ALLOW_LAUNCH=yes");
    }
    // `--live` counts only as an argument to the launcher: agents grep and
    // read the script all day, and a read at night is not a launch.
    for (i, _) in c.match_indices("launch_experiment") {
        let tail = &c[i..];
        let end = ["|", "&&", ";", "
"].iter().filter_map(|t| tail.find(t)).min().unwrap_or(tail.len());
        if tail[..end].contains(" --live") {
            return Some("launch_experiment --live");
        }
    }
    None
}

/// The verb by which a command would move, overwrite, or remove the live
/// database, if it does.
///
/// A word list like the pitch guard: the command names khan.db and carries a
/// verb that moves or destroys files. Reads (sqlite3, python mode=ro, cat,
/// ls, du, `VACUUM INTO` a differently named copy) pass. Copying is refused
/// in both directions — a 200MB copy is the volume filler this all started
/// from, and `cp x khan.db` overwrites in place.
pub fn moves_live_db(command: &str) -> Option<&'static str> {
    let c = command.to_lowercase();
    if !c.contains("khan.db") {
        return None;
    }
    // `> khan.db` and `>khan.db` truncate the open file.
    if c.split('>').skip(1).any(|after| {
        let a = after.trim_start();
        a.starts_with("khan.db") || a.trim_start_matches('/').starts_with("data/khan.db")
    }) {
        return Some(">");
    }
    ["mv", "rename", "rm", "unlink", "truncate", "shred", "dd", "cp", "install", "rsync", "os.replace", "shutil.move", "shutil.copy", "shutil.copyfile"]
        .into_iter()
        .find(|w| contains_word(&c, w))
}

/// The pitch words in an outbound mail command, if it is one.
///
/// Two conditions, both required: the command sends mail, and it carries a
/// word from the banned list. An instruction alone did not hold — a directive
/// landed at 18:07Z on 2026-09-02 and an agent already mid-task mailed
/// hn@ycombinator.com at 18:10 asking about Show HN posts, because the ban
/// reached the CEO and not the dispatch already running.
///
/// Deliberate ceiling: a word list, not an understanding of the mail. It
/// catches the categories the founder named and nothing subtler; a send that
/// names an objective and an amount is treated as transactional and passes.
pub fn pitches_by_mail(command: &str) -> Option<&'static str> {
    let c = command.to_lowercase();
    let sends_mail = ["agentmail", "/messages/send", "smtp", "sendmail", "ses.send", "send_raw_email"]
        .iter()
        .any(|m| c.contains(m));
    if !sends_mail {
        return None;
    }
    // An explicit objective and amount is the founder's own carve-out: a send
    // that names the money it unblocks is transactional by definition.
    let names_objective = c.contains("objective") || c.contains("obj#") || c.contains("obj ");
    if names_objective && c.contains('$') {
        return None;
    }
    // Whole words only. As substrings these are everywhere: "ama" sits inside
    // aiagentsdirectory and amazonses, and on 2026-09-02 it refused a
    // legitimate transactional reply about a listing submission. A word here
    // is bounded by anything that is not a letter or digit.
    ["podcast", "interview", "show hn", "ama", "newsletter", "partnership", "collab", "feature us", "press", "guest post", "sponsor"]
        .into_iter()
        .find(|w| contains_word(&c, w))
}

/// True when `needle` appears in `haystack` bounded by non-alphanumerics on
/// both sides, so "ama" does not match inside "amazonses".
fn contains_word(haystack: &str, needle: &str) -> bool {
    let bytes = haystack.as_bytes();
    haystack.match_indices(needle).any(|(i, _)| {
        let before = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        let end = i + needle.len();
        let after = end >= bytes.len() || !bytes[end].is_ascii_alphanumeric();
        before && after
    })
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
    if let Some(why) = super::fuel_send_blocked(&ctx.store, command) {
        return Ok(why);
    }
    run_with_env(ctx, command, cwd, std::collections::HashMap::new()).await
}
