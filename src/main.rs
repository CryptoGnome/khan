mod agent;
mod config;
mod llm;
mod prompts;
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
        };
        // On Linux the ancestor probe alone let this through: `new` does not exist,
        // so exists() failed all the way back to the workspace and the check passed,
        // then create_dir_all made `new` real and the write escaped.
        for bad in ["new/../../escaped.txt", "../escaped.txt", "a/b/../../../escaped.txt"] {
            assert!(crate::tools::fs::write_file(&ctx, bad, "pwn").is_err(), "{bad} must be rejected");
        }
        assert!(!root.join("escaped.txt").exists(), "nothing may be written outside the workspace");
        assert!(crate::tools::fs::write_file(&ctx, "sub/ok.txt", "fine").is_ok());
        let _ = std::fs::remove_dir_all(&root);
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
