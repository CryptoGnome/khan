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

    let orch = Orchestrator {
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
        llm: llm::Client::new(),
        stop,
        tokens: Default::default(),
    };
    orch.run_ceo(&directive, fresh).await
}

#[cfg(test)]
mod tests {
    use crate::llm::{Client, Message};

    #[test]
    fn request_matches_openai_shape() {
        let msgs = vec![Message::text("system", "s"), Message::text("user", "hi")];
        let schemas = crate::tools::work_schemas();
        let body = Client::build_request("gpt-x", &msgs, &schemas);
        assert_eq!(body["model"], "gpt-x");
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
}
