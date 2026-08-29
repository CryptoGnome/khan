mod agent;
mod config;
mod llm;
mod prompts;
mod routines;
mod state;
mod tools;
mod viewer;

use agent::Orchestrator;
use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "khan", about = "Lightweight autonomous agent orchestrator")]
struct Cli {
    /// Path to config file (env: KHAN_CONFIG)
    #[arg(long, env = "KHAN_CONFIG", default_value = "khan.toml")]
    config: String,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Start a new run with a base directive (replaces any previous run state)
    Run { directive: String },
    /// Resume the previous run where it left off
    Resume,
    /// Deploy-friendly: resume if state exists, else start from the KHAN_DIRECTIVE env var.
    /// If KHAN_DIRECTIVE has changed since, the new directive is adopted without wiping state.
    /// Use this as the container start command so redeploys continue instead of restarting.
    Auto,
    /// Send a message to the running (or next-started) CEO without stopping it
    Tell { message: String },
}

/// Resolves on Ctrl+C (both platforms) or SIGTERM (Linux, e.g. Railway stop/redeploy).
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = signal(SignalKind::terminate()).expect("SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = term.recv() => {},
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // `tell` only needs the DB — works even without API keys configured.
    if let Cmd::Tell { message } = &cli.cmd {
        let store = state::Store::open("khan.db")?;
        store.queue_message(message);
        println!("queued — the CEO will see it on its next iteration");
        return Ok(());
    }

    let cfg = config::Config::load(&cli.config)?;

    let workspace = cfg.workspace_dir();
    std::fs::create_dir_all(&workspace)?;
    let store = Arc::new(state::Store::open("khan.db")?);
    prompts::seed(&store, &cfg);

    let (directive, fresh) = match &cli.cmd {
        Cmd::Run { directive } => {
            store.kv_set("directive", directive);
            store.kv_set("iteration", "0");
            (directive.clone(), true)
        }
        Cmd::Resume => match store.kv_get("directive") {
            Some(d) => (d, false),
            None => bail!("nothing to resume — start with: khan run \"<directive>\""),
        },
        Cmd::Auto => match store.kv_get("directive") {
            // Existing mission: resume — but an edited KHAN_DIRECTIVE takes effect
            // (company state is kept; the CEO is notified via a founder message).
            Some(d) => match std::env::var("KHAN_DIRECTIVE") {
                Ok(nd) if !nd.trim().is_empty() && nd != d => {
                    store.kv_set("directive", &nd);
                    store.queue_message(&format!("Your BASE DIRECTIVE has been replaced. It is now:\n{nd}"));
                    (nd, false)
                }
                _ => (d, false),
            },
            None => {
                let d = std::env::var("KHAN_DIRECTIVE").map_err(|_| {
                    anyhow::anyhow!("no saved mission and KHAN_DIRECTIVE env var not set")
                })?;
                store.kv_set("directive", &d);
                store.kv_set("iteration", "0");
                (d, true)
            }
        },
        Cmd::Tell { .. } => unreachable!("handled above"),
    };

    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = stop.clone();
        tokio::spawn(async move {
            shutdown_signal().await;
            eprintln!("\n[khan] shutdown signal — finishing current step, then saving state...");
            stop.store(true, Ordering::Relaxed);
            // A second signal forces exit; on Railway the platform sends SIGKILL after its grace period.
            shutdown_signal().await;
            eprintln!("[khan] force quit");
            std::process::exit(1);
        });
    }

    println!("khan starting | CEO model: {} | workspace: {}", cfg.ceo_model, workspace.display());
    println!("directive: {directive}\n");

    // Live log viewer (web page + SSE stream). Railway sets PORT for public services.
    let port: u16 = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8080);
    tokio::spawn(viewer::serve(store.clone(), port, workspace.clone()));
    // Scheduled checks (add_routine) run inside the binary at zero model cost;
    // only deviations reach the CEO, as routine alerts.
    tokio::spawn(routines::serve(store.clone(), workspace.clone()));
    store.log("khan", "startup", &format!("CEO model {} | directive: {directive}", cfg.ceo_model));

    // Learn the real free-model caps for this key rather than assuming them: the
    // daily limit is 50 or 1000 depending on whether credits were ever purchased,
    // and khan paces itself against whichever it actually has.
    // Teach the redactor the endpoint's actual value before anything can be logged.
    // It is an ordinary URL, so nothing about its shape gives it away - the log
    // scrubber can only strike it if it knows what it is looking for.
    if let Ok(rpc) = std::env::var("SOLANA_RPC") {
        state::redact_value(&rpc, "RPC");
    }
    // Optional residential proxy for web fetch/search (datacenter IPs are walled
    // off from much of the web). Same contract as SOLANA_RPC: usable by
    // reference, never printable — the URL embeds the proxy credentials.
    let http_proxy = match std::env::var("FETCH_PROXY") {
        Ok(p) if !p.trim().is_empty() => {
            state::redact_value(&p, "FETCH_PROXY");
            match reqwest::Proxy::all(&p) {
                Ok(proxy) => {
                    println!("web fetch: residential proxy configured (FETCH_PROXY)");
                    reqwest::Client::builder()
                        .proxy(proxy)
                        .timeout(std::time::Duration::from_secs(45))
                        .build()
                        .ok()
                }
                Err(e) => {
                    eprintln!("[khan] FETCH_PROXY is set but invalid ({e}) — continuing without a proxy");
                    None
                }
            }
        }
        _ => None,
    };
    let llm = llm::Client::new();
    llm.discover_context_limits(&cfg).await;
    if let Some(l) = llm.discover_limits(&cfg).await {
        let tier = if l.is_free_tier {
            "free tier (no credits ever purchased)"
        } else {
            "paid tier (credits purchased)"
        };
        let msg = format!(
            "OpenRouter {tier}: free models allow {} requests/min and {} requests/day",
            l.rpm, l.rpd
        );
        println!("{msg}");
        store.log("khan", "limits", &msg);
    }

    let orch = Arc::new(Orchestrator {
        // Timeout is essential: a hung web_fetch/web_search would otherwise stall
        // the agent loop forever (search engines throttle datacenter IPs).
        ctx: tools::ToolCtx {
            cfg,
            store,
            workspace,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            http_proxy,
        },
        llm,
        stop,
        tokens: Default::default(),
        pending: Default::default(),
    });
    orch.run_ceo(&directive, fresh).await
}

#[cfg(test)]
mod tests {
    use crate::llm::{Client, Message};

    #[test]
    fn request_matches_openai_shape() {
        let msgs = vec![Message::text("system", "s"), Message::text("user", "hi")];
        let schemas = crate::tools::work_schemas();
        let body = Client::build_request("gpt-x", &msgs, &schemas, 16_384);
        assert_eq!(body["model"], "gpt-x");
        // Must always be sent: gateways impose a small ceiling when it is omitted,
        // and a reasoning model then spends the whole budget thinking and returns
        // an empty answer with finish_reason "length".
        assert_eq!(body["max_tokens"], 16_384);
        // Streamed, because a buffered reply from a slow model sits silent long
        // enough for the gateway edge to time out while the origin is still
        // generating — and the usage chunk is the only place token counts arrive.
        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"]["include_usage"], true);
        assert_eq!(body["messages"][1]["role"], "user");
        assert_eq!(body["messages"][1]["content"], "hi");
        // messages must not serialize null tool fields
        assert!(body["messages"][0].get("tool_calls").is_none());
        // tool schema shape
        let t = &body["tools"][0];
        assert_eq!(t["type"], "function");
        assert!(t["function"]["name"].is_string());
        assert_eq!(t["function"]["parameters"]["type"], "object");
    }

    #[test]
    fn empty_assistant_message_gets_string_content() {
        // A model that returns nothing leaves content and tool_calls both None.
        // Strict providers reject the omitted field ("message.content must be a
        // string"), and such messages already sit in persisted histories — so
        // build_request must backfill "" at request time.
        let empty = Message { role: "assistant".into(), content: None, tool_calls: None, tool_call_id: None, reasoning: None };
        let body = Client::build_request("gpt-x", &[Message::text("user", "hi"), empty], &[], 1024);
        assert_eq!(body["messages"][1]["content"], "");
    }

    #[test]
    fn sensitive_env_vars_are_scrubbed() {
        use crate::tools::shell::is_sensitive_env as sens;
        for name in ["OPENROUTER_API_KEY", "BU0Y_API_KEY", "MY_TOKEN", "MY_SECRET", "DB_PASSWORD", "aws_credential_file"] {
            assert!(sens(name), "{name} should be scrubbed");
        }
        for name in ["PATH", "USERPROFILE", "TEMP", "PYTHONPATH"] {
            assert!(!sens(name), "{name} should survive");
        }
    }

    #[test]
    fn shell_blocks_gh_but_allows_git() {
        use crate::tools::shell::touches_gh as g;
        for c in ["gh auth status", "Get-Date; gh repo list", "echo hi | gh api user", "gh.exe auth token"] {
            assert!(g(c), "{c} should be blocked");
        }
        for c in ["python x.py", "git init", "git commit -m x", "C:/bin/git.exe log", "Get-Content github.md", "echo 'gh is a tool'", "dir"] {
            assert!(!g(c), "{c} should be allowed");
        }
    }

    #[test]
    fn objective_board_ranks_counts_and_flags_missing_plans() {
        let store = crate::state::Store::open(":memory:").unwrap();
        let email = store.add_objective("company email + phone", 1);
        let listings = store.add_objective("listing submissions", 3);
        assert!(store.update_objective(listings, None, None, Some("plan: submit CoinPaprika"), None, None));
        let done = store.add_objective("old bet", 2);
        assert!(store.update_objective(done, None, None, None, None, Some("done")));
        let mut inflight = std::collections::HashMap::new();
        inflight.insert(listings, 2usize);
        let board = store.objectives_board(&inflight);
        // Ordered by rank under the READY header; the done objective is off the board.
        let lines: Vec<&str> = board.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("READY"));
        assert!(lines[1].contains("company email") && lines[1].contains("rank 1"));
        assert!(lines[1].contains("0 task(s) in flight") && lines[1].contains("UNSTAFFED") && lines[1].contains("NO PLAN YET"));
        assert!(lines[2].contains("2 task(s) in flight") && !lines[2].contains("NO PLAN YET") && !lines[2].contains("UNSTAFFED"));
        assert!(!board.contains("BLOCKED"));
        let _ = email;
    }

    #[test]
    fn blocked_objectives_render_apart_and_unblock_when_done() {
        let store = crate::state::Store::open(":memory:").unwrap();
        let phone = store.add_objective("buy phone number", 1);
        let email = store.add_objective("company email", 2);
        let press = store.add_objective("press send", 3);
        assert!(store.set_objective_blockers(email, &format!("#{phone}")));
        assert!(store.set_objective_blockers(press, &format!("{phone},{email}")));
        let board = store.objectives_board(&std::collections::HashMap::new());
        // Blocked section lists both dependents with their blockers; no pressure
        // warnings on blocked lines despite zero staffing.
        assert!(board.contains("BLOCKED"));
        let blocked_part = board.split("BLOCKED").nth(1).unwrap();
        assert!(blocked_part.contains("company email") && blocked_part.contains("buy phone number"));
        assert!(blocked_part.contains("press send"));
        assert!(!blocked_part.contains("UNSTAFFED"));
        // Garbage blockers are ignored rather than blocking forever.
        let junk = store.add_objective("junk-blocked", 4);
        assert!(store.set_objective_blockers(junk, "999,abc"));
        let board = store.objectives_board(&std::collections::HashMap::new());
        assert!(board.split("BLOCKED").next().unwrap().contains("junk-blocked"));
        // Completing phone frees email (its only blocker) but not press (still waits on email).
        assert!(store.update_objective(phone, None, None, None, None, Some("done")));
        let freed = store.newly_ready(phone);
        assert_eq!(freed.len(), 1);
        assert_eq!(freed[0].0, email);
        let board = store.objectives_board(&std::collections::HashMap::new());
        let ready_part = board.split("BLOCKED").next().unwrap().to_string();
        assert!(ready_part.contains("company email"));
        assert!(board.split("BLOCKED").nth(1).unwrap().contains("press send"));
        // Completing email then frees press.
        assert!(store.update_objective(email, None, None, None, None, Some("done")));
        let freed = store.newly_ready(email);
        assert_eq!(freed.len(), 1);
        assert_eq!(freed[0].0, press);
    }

    #[test]
    fn tool_health_reports_only_failing_tools() {
        let store = crate::state::Store::open(":memory:").unwrap();
        for _ in 0..3 {
            store.record_tool_call("shell", true, "");
        }
        store.record_tool_call("web_search", false, "ERROR: search request failed: timed out");
        store.record_tool_call("web_search", false, "ERROR: search request failed: timed out");
        let h = store.tool_health_text();
        assert!(h.contains("web_search"), "should flag the broken tool: {h}");
        assert!(h.contains("2 of 2"), "should report the failure ratio: {h}");
        assert!(!h.contains("shell"), "healthy tools must not add noise: {h}");
    }

    #[test]
    fn manager_flag_survives_saves_and_counts_the_team() {
        let store = crate::state::Store::open(":memory:").unwrap();
        store.save_agent("pm-1", "project manager", "agent:pm-1", "m", "[]");
        store.set_manager("pm-1", true);
        store.save_agent("dev-1", "engineer", "agent:dev-1", "m", "[]");
        assert!(store.is_manager("pm-1"));
        assert!(!store.is_manager("dev-1"), "a plain hire is never a manager");
        // A later save (every task end writes history back) must not demote them.
        store.save_agent("pm-1", "project manager", "agent:pm-1", "m", "[{\"role\":\"user\"}]");
        assert!(store.is_manager("pm-1"), "manager flag survives a history write-back");
        // The hiring ceiling counts active employees, and firing frees a seat.
        assert_eq!(store.count_active_agents(), 2);
        store.fire_agent("dev-1");
        assert_eq!(store.count_active_agents(), 1);
        assert!(!store.is_manager("dev-1"));
    }

    #[test]
    fn routines_schedule_and_alert_flow() {
        let store = crate::state::Store::open(":memory:").unwrap();
        store.upsert_routine("claim-verify", "python3 check.py", 300, "claim rows match chain");
        // Never ran → due immediately; not due again right after a run.
        assert_eq!(store.due_routines(1000), vec![("claim-verify".into(), "python3 check.py".into())]);
        store.mark_routine_run("claim-verify", 1000, "ok");
        assert!(store.due_routines(1100).is_empty(), "not due 100s after a 300s-interval run");
        assert_eq!(store.due_routines(1300).len(), 1, "due again once the interval elapses");
        // Alerts queue for the CEO and drain exactly once.
        store.add_routine_alert("claim-verify", "ALERT: pnl row 40 does not match chain");
        let drained = store.drain_routine_alerts();
        assert_eq!(drained.len(), 1);
        assert!(drained[0].1.contains("row 40"));
        assert!(store.drain_routine_alerts().is_empty(), "alerts deliver once");
        // Removal unschedules.
        assert!(store.delete_routine("claim-verify"));
        assert!(store.due_routines(9999).is_empty());
    }

    #[test]
    fn model_stats_report_latency_and_failures() {
        let store = crate::state::Store::open(":memory:").unwrap();
        store.record_model_call("bu0y/fast", 2_000, true, "");
        store.record_model_call("bu0y/fast", 4_000, true, "");
        store.record_model_call("bu0y/slow", 90_000, true, "");
        store.record_model_call("bu0y/slow", 70_000, false, "429 rate limited");
        let s = store.model_stats_text();
        assert!(s.contains("bu0y/fast: 2 calls, avg 3s"), "avg latency per model: {s}");
        assert!(s.contains("bu0y/slow"), "slow model listed: {s}");
        assert!(s.contains("2 took 60s+"), "counts calls over a minute: {s}");
        assert!(s.contains("1 failed") && s.contains("429"), "failures with last error: {s}");
    }

    #[test]
    fn redact_hides_key_material_but_keeps_public_addresses() {
        use crate::state::redact as r;
        // A Solana secret key (base58, 88 chars) and the JSON keypair file form.
        let secret = "4wBqpZM9xaSheZzJSMawUGeTncBjuLGF1TRHVvz2XjR8DfBBFWnMe1FbGqUyxNhMJ8dCVQoyYuoDVzJvYqVdRaeg";
        let keypair = format!("[{}]", (0..64).map(|i| (i % 256).to_string()).collect::<Vec<_>>().join(","));
        for s in [secret.to_string(), keypair, "sk-or-v1-abcdef0123456789abcdef".into(), "bu0y_abcdef0123456789".into()] {
            let out = r(&format!("writing key {s} to vault"));
            assert!(!out.contains(&s), "secret must not survive redaction: {out}");
            assert!(out.contains("REDACTED"), "should mark the redaction: {out}");
        }
        // The company publishes its deposit address on purpose — a 32-44 char
        // base58 public key must stay readable, or the funding flow breaks.
        let addr = "JmGucHQUPhZsoqnzAGjMkdFDDUDgYtjW3fHjXXu1Lu1";
        assert!(r(&format!("deposit to {addr}")).contains(addr), "public address must survive");
        assert_eq!(r("ran: cargo build --release"), "ran: cargo build --release");
        // A transaction signature is shape-identical to a secret key, so it is
        // redacted too — but the prefix must survive so the on-chain proof of work
        // stays traceable on an explorer.
        let sig = "4vAyncxjU72SMvv7ZUDmkmfscnX6HiKCsoxj1CPBeiaE4v59LJ4MwmrgXR2hwyhEP7F7hrNRcpcB7ed2Jt2AkMAE";
        let out = r(&format!("confirmed {sig}"));
        assert!(out.contains("4vAync"), "signature prefix must survive: {out}");
        assert!(!out.contains(sig), "full 88-char token must not survive: {out}");
    }

    #[test]
    fn model_reasoning_is_captured_but_never_sent_back() {
        use crate::llm::Message;
        // Providers disagree on the field name; both must land.
        for field in ["reasoning", "reasoning_content"] {
            let raw = format!(
                r#"{{"role":"assistant","content":null,"{field}":"I should check the balance first."}}"#
            );
            let m: Message = serde_json::from_str(&raw).unwrap();
            assert_eq!(
                m.reasoning.as_deref(),
                Some("I should check the balance first."),
                "{field} should be captured"
            );
            // Echoing reasoning back upstream can be rejected, so it must not serialize.
            let back = serde_json::to_string(&m).unwrap();
            assert!(!back.contains("reasoning"), "reasoning must not be sent back: {back}");
            assert!(!back.contains("check the balance"), "reasoning text must not be sent back");
        }
        // A message with no reasoning field at all still parses.
        let plain: Message = serde_json::from_str(r#"{"role":"user","content":"hi"}"#).unwrap();
        assert!(plain.reasoning.is_none());
    }

    #[test]
    fn compaction_keeps_recent_turns_and_never_orphans_a_tool_result() {
        use crate::agent::split_point;
        use crate::llm::Message;
        let m = |role: &str, n: usize| Message::text(role, "x".repeat(n));

        // System prompt plus 10 exchanges of 1000 chars each. Keeping 2500 chars
        // should retain the last few messages and cut the rest.
        let mut h = vec![m("system", 500)];
        for _ in 0..10 {
            h.push(m("assistant", 1000));
            h.push(m("user", 1000));
        }
        let s = split_point(&h, 2500);
        assert!(s > 1, "must never summarize away the system prompt");
        assert!(s < h.len(), "must keep some recent context");
        let kept: usize = h[s..].iter().map(|x| x.content.as_deref().unwrap().len()).sum();
        assert!(kept >= 2500, "should bank at least the requested recency, got {kept}");

        // A tool result must not become the first kept message — it would be an
        // answer to a call the model can no longer see.
        let mut h2 = vec![m("system", 100), m("user", 5000), m("assistant", 100)];
        h2.push(Message::tool_result("c1", "y".repeat(50)));
        h2.push(m("assistant", 50));
        let s2 = split_point(&h2, 100);
        assert_ne!(h2[s2].role, "tool", "kept tail must not open on a tool result");

        // A history shorter than the budget keeps everything after the system prompt.
        let h3 = vec![m("system", 10), m("user", 10), m("assistant", 10)];
        assert_eq!(split_point(&h3, 100_000), 1);
    }

    #[test]
    fn rate_limit_reset_header_is_understood() {
        use reqwest::header::{HeaderMap, HeaderValue};
        let hm = |k: &'static str, v: String| {
            let mut h = HeaderMap::new();
            h.insert(k, HeaderValue::from_str(&v).unwrap());
            h
        };
        let secs = |h: HeaderMap| crate::llm::retry_after(&h).map(|d| d.as_secs() as i64);
        let now_ms = chrono::Utc::now().timestamp_millis();

        // Retry-After is plain seconds.
        assert_eq!(secs(hm("retry-after", "30".into())), Some(30));
        // X-RateLimit-Reset arrives as an epoch in ms, an epoch in seconds, or a delta.
        let ms = secs(hm("x-ratelimit-reset", (now_ms + 60_000).to_string())).unwrap();
        assert!((58..=61).contains(&ms), "epoch ms should be ~60s, got {ms}");
        let s = secs(hm("x-ratelimit-reset", (now_ms / 1000 + 120).to_string())).unwrap();
        assert!((118..=121).contains(&s), "epoch seconds should be ~120s, got {s}");
        assert_eq!(secs(hm("x-ratelimit-reset", "45".into())), Some(45));
        // A reset already in the past, and no header at all, mean no wait.
        assert_eq!(secs(hm("x-ratelimit-reset", (now_ms - 60_000).to_string())), None);
        assert_eq!(secs(HeaderMap::new()), None);
        // Never park a model for longer than a day on a bogus value.
        let huge = secs(hm("retry-after", "999999999".into())).unwrap();
        assert_eq!(huge, 86_400);
    }

    #[test]
    fn api_keys_leave_the_process_environment_at_load() {
        // Stripping child envs is not enough: a child can read its parent's copy
        // from /proc/<pid>/environ on Linux. The key must not be in the parent's
        // environment at all once config is loaded.
        std::env::set_var("KHAN_TEST_API_KEY", "super-secret-value");
        let path = std::env::temp_dir().join("khan-key-scrub-test.toml");
        std::fs::write(
            &path,
            "ceo_model = \"p/m\"\n[[providers]]\nname = \"p\"\nbase_url = \"http://x\"\napi_key_env = \"KHAN_TEST_API_KEY\"\npaid_models = [\"m\"]\n",
        )
        .unwrap();
        let cfg = crate::config::Config::load(path.to_str().unwrap()).unwrap();

        assert!(
            std::env::var("KHAN_TEST_API_KEY").is_err(),
            "the key must be gone from the environment after load"
        );
        // ...and khan must still be able to use it.
        let (_, model, key) = cfg.resolve("p/m").unwrap();
        assert_eq!(model, "m");
        assert_eq!(key, "super-secret-value");
        assert_eq!(cfg.key_for("p"), Some("super-secret-value"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn workspace_paths_cannot_escape_via_parent_dir() {
        let cfg: crate::config::Config = toml::from_str(
            "ceo_model = \"p/m\"\n[[providers]]\nname = \"p\"\nbase_url = \"http://x\"\napi_key_env = \"X\"\npaid_models = [\"m\"]\n",
        )
        .unwrap();
        let root = std::env::temp_dir().join("khan-fs-escape-test");
        let ws = root.join("ws");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&ws).unwrap();
        let ctx = crate::tools::ToolCtx {
            cfg,
            store: std::sync::Arc::new(crate::state::Store::open(":memory:").unwrap()),
            workspace: ws.clone(),
            http: reqwest::Client::new(),
            http_proxy: None,
        };
        // On Linux the ancestor probe alone let this through: `new` does not exist,
        // so exists() failed all the way back to the workspace and the check passed,
        // then create_dir_all made `new` real and the write escaped.
        for bad in ["new/../../escaped.txt", "../escaped.txt", "a/b/../../../escaped.txt"] {
            assert!(crate::tools::fs::write_file(&ctx, bad, "pwn", false).is_err(), "{bad} must be rejected");
            // Appending creates the file too, so it must be contained just as tightly.
            assert!(crate::tools::fs::write_file(&ctx, bad, "pwn", true).is_err(), "{bad} must be rejected on append");
        }
        assert!(!root.join("escaped.txt").exists(), "nothing may be written outside the workspace");
        assert!(crate::tools::fs::write_file(&ctx, "sub/ok.txt", "fine", false).is_ok());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A truncated answer must be recognisable as such, because the three callers
    /// each treat it differently from an ordinary failure: no model fallback, no
    /// dead employee, no silent CEO retry.
    #[test]
    fn truncation_is_distinguishable_from_an_ordinary_failure() {
        use crate::llm::{truncation, Truncated};
        let t = anyhow::Error::new(Truncated { max_tokens: 16_384, reasoning_tokens: 16_000 })
            .context("openrouter/some-model");
        let got = truncation(&t).expect("must survive being wrapped in context");
        assert_eq!(got.max_tokens, 16_384);
        assert_eq!(got.reasoning_tokens, 16_000);
        // The message still has to read well in the log, model included.
        let shown = format!("{t:#}");
        assert!(shown.contains("openrouter/some-model"), "{shown}");
        assert!(shown.contains("16384-token output budget"), "{shown}");
        // Anything else must not be mistaken for it.
        assert!(truncation(&anyhow::anyhow!("429 rate limited")).is_none());
    }

    /// The ceiling is per-model where the provider publishes one, and the same
    /// number compaction reserves — the two must not drift apart.
    #[test]
    fn output_ceiling_follows_the_model_not_one_global_number() {
        use crate::agent::compact_threshold;
        let mut cfg: crate::config::Config = toml::from_str(
            "ceo_model = \"p/m\"\n[[providers]]\nname = \"p\"\nbase_url = \"http://x\"\napi_key_env = \"X\"\npaid_models = [\"m\"]\n",
        )
        .unwrap();
        cfg.max_tokens = 16_384;
        let c = Client::new();
        // Unknown model: the configured default, unchanged from before.
        assert_eq!(c.output_limit("bu0y/whatever", &cfg), 16_384);
        c.set_output_limit("openrouter/deepseek/deepseek-v3.2", 65_536);
        c.set_output_limit("openrouter/minimax/minimax-m3", 512_000);
        c.set_output_limit("openrouter/minimax/minimax-m2-her", 2_048);
        // Published and modest: taken as-is, four times the old fixed ceiling.
        assert_eq!(c.output_limit("openrouter/deepseek/deepseek-v3.2", &cfg), 65_536);
        // Published and enormous: capped, or compaction would reserve the window.
        assert_eq!(c.output_limit("openrouter/minimax/minimax-m3", &cfg), 65_536);
        // Published and small: we stop over-asking, which the old global did.
        assert_eq!(c.output_limit("openrouter/minimax/minimax-m2-her", &cfg), 2_048);
        // A bigger reserve must never loosen compaction.
        let ctx = Some(163_840);
        assert!(compact_threshold(ctx, 65_536) <= compact_threshold(ctx, 16_384));
    }

    #[test]
    fn append_builds_a_file_across_several_writes() {
        use crate::tools::fs::write_file;
        let cfg: crate::config::Config = toml::from_str(
            "ceo_model = \"p/m\"\n[[providers]]\nname = \"p\"\nbase_url = \"http://x\"\napi_key_env = \"X\"\npaid_models = [\"m\"]\n",
        )
        .unwrap();
        let root = std::env::temp_dir().join("khan-append-test");
        let ws = root.join("ws");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&ws).unwrap();
        let ctx = crate::tools::ToolCtx {
            cfg,
            store: std::sync::Arc::new(crate::state::Store::open(":memory:").unwrap()),
            workspace: ws.clone(),
            http: reqwest::Client::new(),
            http_proxy: None,
        };
        // Appending to a file that does not exist yet must create it, so an agent
        // can start a chunked write without a separate setup call.
        write_file(&ctx, "page.html", "<html>", true).unwrap();
        write_file(&ctx, "page.html", "body", true).unwrap();
        let out = write_file(&ctx, "page.html", "</html>", true).unwrap();
        assert_eq!(std::fs::read_to_string(ws.join("page.html")).unwrap(), "<html>body</html>");
        // The running total is what tells the agent how far along it is.
        assert!(out.contains("17 bytes total"), "{out}");
        // Overwrite still truncates: append must be opt-in, never the default.
        write_file(&ctx, "page.html", "fresh", false).unwrap();
        assert_eq!(std::fs::read_to_string(ws.join("page.html")).unwrap(), "fresh");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The live failure this guards: ten self-rewrites had compressed the CEO's
    /// prompt to an operations manual that kept "re-home a slow hire" and had lost
    /// every word about growing the company — so a directive demanding an org
    /// chart met a CEO whose own prompt never mentioned hiring anyone new.
    #[test]
    fn ceo_mandate_survives_a_prompt_that_evolved_it_away() {
        use crate::prompts::ceo_system;
        let evolved = "You are the CEO. Re-home hires whose latency >60s avg or failures rise.";
        let sys = ceo_system(evolved);
        assert!(sys.contains(evolved), "the CEO's own prompt still comes first");
        for must in [
            "You DIRECT; you do not execute",
            // Planning is the loophole the old wording left open: "quick checks
            // and decisions are yours" grew until it covered every iteration.
            "commission it and judge it",
            "doing the job twice",
            "a team of four is not a company",
            "hire(manager: true)",
            "PROGRESS",
            "never park it for",
            // Capability matching has to outlive prompt evolution too: the seeded
            // "plan with power, execute cheap" line was compressed away long ago.
            "Match the model to the stakes",
            "untested, not bad",
            // Skills and memories kept beating the mandate: a self-written
            // procedure naming the CEO as the one who runs a step is specific,
            // and specific was winning against a general rule every time.
            "outranks anything you wrote yourself",
            "REWRITE IT NOW",
        ] {
            assert!(sys.contains(must), "mandate lost {must}");
        }
        // The two code-sourced blocks must coexist; neither displaces the other.
        assert!(sys.contains("SECURITY RULES"), "security must still apply");
        // A wiped or missing prompt still carries the mandate: get_prompt returns
        // the empty string when the row is gone, and that path must not be bare.
        assert!(ceo_system("").contains("STANDING MANDATE"));
    }

    /// Idle capacity was the one thing no block reported: the company sat at four
    /// employees with thirty-six seats free and read as healthy by every measure
    /// it had. This makes headcount and silence measurable.
    #[test]
    fn team_capacity_reports_seats_free_and_who_has_gone_quiet() {
        let store = crate::state::Store::open(":memory:").unwrap();
        // No employees is itself worth saying — that is the CEO doing all the work.
        assert!(store.team_capacity_text(40).contains("No employees at all"));
        store.save_agent("scout", "researcher", "agent:scout", "m", "[]");
        store.save_agent("idler", "does nothing yet", "agent:idler", "m", "[]");
        store.log("scout", "shell", "looked something up");
        let text = store.team_capacity_text(40);
        assert!(text.contains("2 employees, ceiling 40 — 38 seats free"), "{text}");
        assert!(text.contains("scout: silent 0m"), "{text}");
        // A hire that has never run at all must not read as freshly active.
        assert!(text.contains("idler: has never done anything"), "{text}");
        // Handing out no work at all is the state worth naming, and it needs no
        // judgement about what counts as progress.
        assert!(text.contains("never started work through anyone"), "{text}");
        store.log("CEO", "dispatch", "gave the scout a task");
        assert!(
            store.team_capacity_text(40).contains("started new work through anyone"),
            "a dispatch must reset it to an elapsed time"
        );
        // The CEO is not an employee and never counts against the seats.
        store.log("CEO", "says", "thinking");
        assert!(!store.team_capacity_text(40).contains("CEO:"), "CEO is not a seat");
        // Firing frees the seat back up.
        store.fire_agent("idler");
        assert!(store.team_capacity_text(40).contains("39 seats free"));
    }

    /// The CEO having the no-handoff rule was not enough: it dispatched "build a
    /// founder-followable day-of runbook" and the webmaster built it, having
    /// nothing in its own context to object with.
    #[test]
    fn employees_are_told_they_have_no_founder_to_hand_work_to() {
        use crate::prompts::{ceo_system, employee_system};
        let sys = employee_system("You are {name}, a webmaster.");
        for must in [
            "no founder to hand work to",
            "never deliver a checklist",
            "A wall counts only once you have actually hit it",
        ] {
            assert!(sys.contains(must), "worker mandate lost {must}");
        }
        assert!(sys.contains("SECURITY RULES"), "employees keep the security rules");
        // The employee mandate is not the CEO's: an employee must not be told to
        // hire, staff up or run two tracks — that is the CEO's job, and handing it
        // to everyone would have the whole company trying to run the company.
        for ceo_only in ["hire(manager: true)", "a team of four is not a company", "You DIRECT"] {
            assert!(!sys.contains(ceo_only), "employee must not get CEO clause {ceo_only}");
            assert!(ceo_system("x").contains(ceo_only), "CEO must still have {ceo_only}");
        }
    }

    /// The old ladder sent every failing agent to the free list, so the CEO — and
    /// any employee deliberately put on an expensive model because the job was
    /// hard — got silently demoted to a free model at the exact moment its own
    /// model had just failed. Free models are for work already judged cheap.
    #[test]
    fn a_failing_paid_model_never_falls_back_to_a_free_one() {
        let cfg: crate::config::Config = toml::from_str(
            "ceo_model = \"pay/big\"\n\
             [[providers]]\nname = \"pay\"\nbase_url = \"http://x\"\napi_key_env = \"X\"\n\
             paid_models = [\"big\", \"small\"]\n\
             [[providers]]\nname = \"or\"\nbase_url = \"http://y\"\napi_key_env = \"Y\"\n\
             free_models = [\"cheap:free\"]\n",
        )
        .unwrap();
        let paid = cfg.fallback_ids_for("pay/big");
        assert!(!paid.iter().any(|m| m.contains(":free")), "paid must not fall to free: {paid:?}");
        assert!(paid.contains(&"pay/small".to_string()), "{paid:?}");
        // A free agent is already doing work judged cheap, so its ladder is unchanged.
        let free = cfg.fallback_ids_for("or/cheap:free");
        assert_eq!(free, vec!["or/cheap:free".to_string()]);
        // A model hired off-catalog (the CEO may name any slug) counts as paid.
        assert_eq!(cfg.fallback_ids_for("pay/unlisted"), cfg.paid_model_ids());
    }

    /// An evidence-only policy can never reach a new model: the incumbent has
    /// thousands of good calls and the newcomer has none, so it loses every
    /// comparison however capable it is. The gap has to be named to be closed.
    #[test]
    fn models_never_called_are_reported_as_unmeasured() {
        let cfg: crate::config::Config = toml::from_str(
            "ceo_model = \"pay/big\"\n\
             [[providers]]\nname = \"pay\"\nbase_url = \"http://x\"\napi_key_env = \"X\"\n\
             paid_models = [\"big\", \"fresh\"]\n\
             [[providers]]\nname = \"or\"\nbase_url = \"http://y\"\napi_key_env = \"Y\"\n\
             free_models = [\"cheap:free\"]\n",
        )
        .unwrap();
        let store = crate::state::Store::open(":memory:").unwrap();
        store.record_model_call("pay/big", 1200, true, "");
        let seen = store.models_seen();
        assert_eq!(seen, vec!["pay/big".to_string()]);
        let untried = cfg.untried_models(&seen);
        assert!(!untried.contains(&"pay/big".to_string()), "a measured model is not untried");
        assert!(untried.contains(&"pay/fresh".to_string()), "{untried:?}");
        // Free models count too — an untried free model is also unmeasured.
        assert!(untried.contains(&"or/cheap:free".to_string()), "{untried:?}");
        // A failed call is still a measurement: it produced data either way.
        store.record_model_call("pay/fresh", 500, false, "boom");
        assert!(!cfg.untried_models(&store.models_seen()).contains(&"pay/fresh".to_string()));
    }

    #[test]
    fn tool_call_roundtrip() {
        let raw = r#"{"role":"assistant","content":null,
            "tool_calls":[{"id":"c1","type":"function","function":{"name":"shell","arguments":"{\"command\":\"dir\"}"}}]}"#;
        let m: Message = serde_json::from_str(raw).unwrap();
        let calls = m.tool_calls.as_ref().unwrap();
        assert_eq!(calls[0].function.name, "shell");
        let back = serde_json::to_string(&m).unwrap();
        assert!(back.contains("\"tool_calls\""));
    }

    #[test]
    fn context_aware_compaction_can_only_ever_tighten() {
        use crate::agent::{compact_threshold, Orchestrator as O};
        let (at, floor) = (O::COMPACT_AT, O::COMPACT_FLOOR);

        // An unknown window - every bu0y model, since their catalog is prices only -
        // must leave the existing threshold untouched.
        assert_eq!(compact_threshold(None, 16_384), at);

        // The models actually configured today all have room to spare, so the
        // context-aware path must be a no-op for them.
        for ctx in [256_000u32, 1_000_000, 1_048_576] {
            assert_eq!(compact_threshold(Some(ctx), 16_384), at, "ctx {ctx} should not change");
        }

        // A window too small for a 200k history tightens, and by enough to matter.
        assert!(compact_threshold(Some(64_000), 16_384) < at);

        // Sweep the whole plausible space, including the degenerate cases that
        // would panic or wrap on unchecked arithmetic: never above the old
        // threshold, never below the anti-thrash floor, and always leaving room
        // for compaction to actually get under the bar.
        for ctx in [1u32, 2, 1_000, 8_192, 32_768, 64_000, 128_000, 200_000, u32::MAX] {
            for mt in [1u32, 4_096, 16_384, 100_000, u32::MAX] {
                let got = compact_threshold(Some(ctx), mt);
                assert!(got <= at, "ctx {ctx} mt {mt}: {got} exceeds {at}");
                assert!(got >= floor, "ctx {ctx} mt {mt}: {got} below {floor}");
                assert!(got > O::KEEP_RECENT, "ctx {ctx} mt {mt}: would thrash");
            }
        }
    }

    #[test]
    fn registered_endpoint_never_survives_into_the_log() {
        use crate::state::{redact as r, redact_value};
        // Shaped exactly like a paid RPC endpoint: the secret is the key in the query.
        let rpc = "https://mainnet.example-rpc.com/?api-key=9fK2xQ7bLp4RtY8wZ3nA6vC1mS0dJhGe";
        let key = "9fK2xQ7bLp4RtY8wZ3nA6vC1mS0dJhGe";
        redact_value(rpc, "RPC");

        // The obvious leak: the agent echoes the variable.
        assert!(!r(&format!("$ echo $SOLANA_RPC
{rpc}")).contains(rpc));
        // Buried in a command line, which is what actually reaches the log.
        assert!(!r(&format!("{{\"command\": \"curl -s {rpc} -d '{{}}'\"}}")).contains(rpc));
        // And the key alone, without the URL around it.
        assert!(!r(&format!("using key {key} for the call")).contains(key));
        // A stack trace or error text quoting the endpoint back at us.
        assert!(!r(&format!("ConnectionError: failed to reach {rpc} after 3 tries")).contains(rpc));

        // The label is what shows instead, so the log still reads sensibly.
        assert!(r(rpc).contains("[REDACTED-RPC]"));

        // The host is NOT secret and must stay legible - blanking every mention of
        // a hostname would gut the log without protecting anything.
        assert!(r("connected to mainnet.example-rpc.com").contains("mainnet.example-rpc.com"));

        // Too-short values are refused outright: registering one would redact
        // ordinary English out of every line.
        redact_value("abc", "NOPE");
        assert_eq!(r("abc is a normal word fragment"), "abc is a normal word fragment");
    }
}
