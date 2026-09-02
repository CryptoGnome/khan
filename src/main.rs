mod agent;
mod config;
mod llm;
mod prompts;
mod routines;
mod telegram;
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
    tools::skills::seed(&store);

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
    // Crash-loop tripwire: a healthy process starts once per deploy. Three
    // startups inside 15 minutes means the binary is dying on its own, and
    // the founder hears it from the binary directly — no model in the loop
    // (the 2026-08-31 em-dash panic looped for 13 minutes before a human
    // noticed by accident).
    let recent = store.recent_startup_count(15 * 60);
    if recent >= 3 {
        if let Some((token, chat)) = cfg.telegram() {
            let msg = format!(
                "⚠️ khan is crash-looping: {recent} startups in the last 15 minutes. \
The binary is dying and restarting on its own — check the Railway deploy logs. \
Episodes lose their in-flight dispatches on every crash."
            );
            let http = reqwest::Client::new();
            if telegram::send(&http, &token, chat, &msg).await.is_err() {
                eprintln!("[khan] crash-loop alert could not reach Telegram");
            }
        }
        store.log("khan", "crash-loop", &format!("{recent} startups in 15m — founder alerted"));
    }

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
        seat: Default::default(),
        running: Default::default(),
    });
    // Scheduled checks run inside the binary: shell routines at zero model
    // cost, review routines as scheduled dispatches through the orchestrator.
    // Only deviations (and review reports) reach the CEO.
    // The founder's direct line: Telegram in (queued like `khan tell`),
    // message_founder out. Only spawned when both env halves are set.
    if let Some((token, chat)) = orch.ctx.cfg.telegram() {
        tokio::spawn(telegram::serve(orch.ctx.store.clone(), orch.ctx.http.clone(), token, chat));
    }
    // X Activity stream: mention events push instead of paid polling. The
    // task no-ops instantly when X credentials are not configured.
    tokio::spawn(tools::x::activity_stream(orch.ctx.clone()));
    tokio::spawn(routines::serve(
        orch.ctx.store.clone(),
        orch.ctx.workspace.clone(),
        orch.clone(),
    ));
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
        // only the newest picture is expanded into content parts; the earlier
        // one was seen on its own turn and would only grow the body toward 413
        let shot = |n: &str| Message { images: Some(vec![format!("data:image/png;base64,{n}")]), ..Message::text("user", n) };
        let pic = Client::build_request("glm53flash", &[shot("first"), Message::text("assistant", "ok"), shot("second")], &[], 1024);
        assert_eq!(pic["messages"][0]["content"], "first");
        assert_eq!(pic["messages"][2]["content"][1]["type"], "image_url");
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
        let empty = Message { role: "assistant".into(), content: None, tool_calls: None, tool_call_id: None, reasoning: None, images: None };
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
        // Custom-tool scripts are scanned with the same check at create time —
        // it must catch a gh call buried mid-script, line by line.
        assert!(g("echo start\ngh api user\necho end"), "multi-line script with gh line should be blocked");
        assert!(!g("import json, os\nargs = json.loads(os.environ['KHAN_TOOL_ARGS'])\nprint(args)"), "benign python script should pass");
    }

    #[test]
    fn oversized_tool_output_spills_to_workspace() {
        use crate::tools::{truncate_spill, MAX_RESULT};
        let dir = std::env::temp_dir().join(format!("khan-spill-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Short output passes through untouched, no spill dir created.
        assert_eq!(truncate_spill(&dir, "shell", "small".into()), "small");
        assert!(!dir.join(".spill").exists());
        // Oversized output is cut but the full text survives in .spill/ and the
        // marker names the file so an agent can read_file the rest. The TAIL is
        // what stays visible — errors land at the end of output.
        let big = format!("HEAD-{}-TAIL", "x".repeat(MAX_RESULT + 500));
        let out = truncate_spill(&dir, "shell", big.clone());
        assert!(out.len() < big.len());
        assert!(out.starts_with("[truncated"), "marker leads the kept slice: {}", &out[..80]);
        assert!(out.contains("full output saved to .spill/"), "marker should name the spill file: {}", &out[..160]);
        assert!(out.ends_with("-TAIL") && !out.contains("HEAD-"), "the end of shell output must stay visible");
        // Document-like tools keep the HEAD: an oversized web page must not
        // lose its leading BEGIN-UNTRUSTED marker to a tail cut.
        let page = format!("[BEGIN UNTRUSTED WEB CONTENT]{}", "y".repeat(MAX_RESULT + 500));
        let out = truncate_spill(&dir, "web_fetch", page);
        assert!(out.starts_with("[BEGIN UNTRUSTED WEB CONTENT]"), "untrusted marker must survive truncation");
        let spilled = std::fs::read_dir(dir.join(".spill")).unwrap().next().unwrap().unwrap();
        assert_eq!(std::fs::read_to_string(spilled.path()).unwrap(), big);
        // The directory cleans itself: a generous max_age keeps the file, a
        // zero max_age (everything is "old") removes it.
        crate::tools::purge_spill(&dir.join(".spill"), std::time::Duration::from_secs(3600));
        assert!(std::fs::read_dir(dir.join(".spill")).unwrap().next().is_some(), "fresh spill must survive the sweep");
        crate::tools::purge_spill(&dir.join(".spill"), std::time::Duration::ZERO);
        assert!(std::fs::read_dir(dir.join(".spill")).unwrap().next().is_none(), "aged-out spill must be removed");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn x_ledger_tracks_budget_and_alerts_once_when_low() {
        let store = crate::state::Store::open(":memory:").unwrap();
        // Seeded with the founder's $5 exactly once.
        assert!((store.x_balance() - 5.0).abs() < 1e-9);
        // Spends debit; the first dip under $1 raises ONE alert, not one per call.
        store.x_debit(0.015, "x_post 111");
        assert!((store.x_balance() - 4.985).abs() < 1e-9);
        store.x_debit(4.0, "x_post burst");
        store.x_debit(0.005, "activity event");
        let alerts = store.drain_routine_alerts();
        assert_eq!(alerts.iter().filter(|(n, _)| n == "x-budget").count(), 1, "low-balance alert fires once");
        assert!(alerts.iter().any(|(_, d)| d.contains("x_topup")), "alert carries the top-up path");
        // A verified top-up credits, dedups by tx signature, and re-arms the alert.
        store.x_topup_credit(3.0, "USDC top-up, tx 5sig9");
        assert!((store.x_balance() - 3.98).abs() < 1e-6);
        assert!(store.x_ledger_has("5sig9"));
        assert!(!store.x_ledger_has("othersig"));
        store.x_debit(3.5, "x_post big day");
        assert_eq!(store.drain_routine_alerts().iter().filter(|(n, _)| n == "x-budget").count(), 1, "top-up re-arms the alert");
        let tail = store.x_ledger_tail(3);
        assert_eq!(tail.len(), 3);
        assert!(tail[0].contains("-$3.500"), "newest first with signed amounts: {}", tail[0]);
    }

    #[test]
    fn ceo_execution_budget_classifies_doing_vs_directing() {
        use crate::agent::ceo_exec_budgeted as b;
        // Doing: shell, sql, and custom registry tools (an employee can run
        // every one of them — sends and kill-exits included — so nothing is
        // lost at the cap).
        for t in ["shell", "sql", "sol_send", "pump_sell", "page_health", "khan_db_query"] {
            assert!(b(t), "{t} must count against the execution budget");
        }
        // Directing and reading stay unlimited.
        for t in ["dispatch", "delegate", "rate_work", "team_status", "objectives", "finish_episode",
                  "message_founder", "read_file", "list_files", "recall", "web_fetch", "web_search",
                  "use_skill", "credits", "x_post", "x_read"] {
            assert!(!b(t), "{t} must stay unbudgeted");
        }
    }

    #[test]
    fn x_ledger_mirrors_the_real_billing_rules() {
        // X bills per distinct resource per UTC day (dedup verified against
        // docs.x.com 2026-09-01): first sighting counts, a re-read the same
        // day is free, a new day charges again. Without this the ledger
        // drifts pessimistic and strands prepaid credits at a fake $0.
        let dir = std::env::temp_dir().join(format!("khan-xseen-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = crate::state::Store::open(dir.join("khan.db").to_str().unwrap()).unwrap();
        assert_eq!(store.x_mark_seen(&["t1", "t2", "t3"], "2026-09-01"), 3);
        assert_eq!(store.x_mark_seen(&["t1", "t2"], "2026-09-01"), 0, "same-day re-read is free");
        assert_eq!(store.x_mark_seen(&["t2", "t4"], "2026-09-01"), 1, "only the unseen one bills");
        assert_eq!(store.x_mark_seen(&["t1"], "2026-09-02"), 1, "the window resets at midnight UTC");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn crew_brief_shows_roster_state_and_pushes_fanout() {
        // A manager opens every task seeing who it can delegate to and who is
        // taken — the 2026-09-01 audit found nine managers issued one
        // delegate_parallel between them because the roster was invisible.
        let rows = vec![
            ("eng-dev".into(), "bu0y/glm53flash".into(), "Developer for the build lane".into(), false, false),
            ("builder".into(), "bu0y/glm53flash".into(), "Builder/engineer".into(), false, true),
            ("cfo".into(), "bu0y/glm53flash".into(), "Chief Financial Officer".into(), true, false),
        ];
        let brief = crate::agent::crew_brief(&rows);
        assert!(brief.contains("eng-dev") && brief.contains("(idle)"), "{brief}");
        assert!(brief.contains("builder") && brief.contains("(BUSY)"), "{brief}");
        assert!(brief.contains("cfo") && brief.contains("(manager)"), "{brief}");
        assert!(brief.contains("delegate_parallel"), "{brief}");
        assert!(brief.contains("hire"), "{brief}");
    }

    #[test]
    fn js_shell_pages_are_detected_and_static_pages_are_not() {
        // sharc.fun: 3,199 bytes of HTML, 38 chars of visible text, one Vite
        // bundle — reported as a successful fetch, so the site got dropped
        // from the style study for "rendering empty".
        let shell = r#"<!doctype html><html><head><link rel="stylesheet" href="/assets/index-DOBJDI_t.css"></head>
<body><div id="root"></div><script type="module" src="/assets/index-B16IOJ0c.js"></script></body></html>"#;
        let text = html2text::from_read(shell.as_bytes(), 100);
        assert!(crate::tools::web::looks_like_js_shell(shell, &text));
        let assets = crate::tools::web::assets(shell, "https://sharc.fun/");
        assert_eq!(assets, vec!["https://sharc.fun/assets/index-DOBJDI_t.css", "https://sharc.fun/assets/index-B16IOJ0c.js"]);

        let article = format!("<html><body><article>{}</article><script src=\"/a.js\"></script></body></html>", "real words ".repeat(60));
        let text = html2text::from_read(article.as_bytes(), 100);
        assert!(!crate::tools::web::looks_like_js_shell(&article, &text), "a page with real text is not a shell");
        let bare = "<html><body><p>tiny</p></body></html>";
        assert!(!crate::tools::web::looks_like_js_shell(bare, "tiny"), "a tiny static page has no bundle to render");
    }

    #[test]
    fn page_dates_come_from_meta_tags_and_json_ld() {
        let html = r#"<head>
<meta property="article:published_time" content="2026-08-30T14:02:00Z">
<meta property="og:updated_time" content="2026-09-01T09:00:00Z">
<script type="application/ld+json">{"@type":"NewsArticle","datePublished":"2026-08-30","author":"x"}</script>
</head>"#;
        let d = crate::tools::web::page_dates(html);
        assert!(d.iter().any(|x| x == "article:published_time=2026-08-30T14:02:00Z"), "{d:?}");
        assert!(d.iter().any(|x| x == "og:updated_time=2026-09-01T09:00:00Z"), "{d:?}");
        assert!(d.iter().any(|x| x == "datepublished=2026-08-30"), "{d:?}");
        assert!(crate::tools::web::page_dates("<html><body>no dates here</body></html>").is_empty());
    }

    #[test]
    fn images_become_content_parts_only_for_vision_seats() {
        use crate::llm::{Client, Message};
        let msgs = vec![
            Message::text("user", "hello"),
            Message::with_images("user", "look", vec!["data:image/png;base64,AAAA".into()]),
        ];
        let body = Client::build_request("glm53flash", &msgs, &[], 100);
        let m = &body["messages"];
        assert_eq!(m[0]["content"], "hello", "plain messages stay strings");
        let parts = m[1]["content"].as_array().expect("image message becomes parts");
        assert_eq!(parts[0]["type"], "text");
        assert_eq!(parts[1]["type"], "image_url");
        assert_eq!(parts[1]["image_url"]["url"], "data:image/png;base64,AAAA");
        // a text-only fallback seat gets the text alone rather than a 400
        let body = Client::build_request("deepseekv4flash", &msgs, &[], 100);
        assert_eq!(body["messages"][1]["content"], "look");
        // and the field never rides on the wire as its own key
        assert!(body["messages"][1].get("images").is_none());
    }

    #[test]
    fn image_marker_results_produce_a_picture_turn() {
        let cfg: crate::config::Config = toml::from_str(
            "ceo_model = \"p/m\"\n[[providers]]\nname = \"p\"\nbase_url = \"http://x\"\napi_key_env = \"X\"\npaid_models = [\"m\"]\n",
        )
        .unwrap();
        let root = std::env::temp_dir().join("khan-image-followup-test");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let ctx = crate::tools::ToolCtx {
            cfg,
            store: std::sync::Arc::new(crate::state::Store::open(":memory:").unwrap()),
            workspace: root.clone(),
            http: reqwest::Client::new(),
            http_proxy: None,
        };
        // 1x1 PNG header is enough: the follow-up inlines bytes, it does not decode them.
        std::fs::write(root.join("shot.png"), [0x89, b'P', b'N', b'G', 0, 1, 2, 3]).unwrap();
        let out = crate::tools::view_image(&ctx, "shot.png").unwrap();
        assert!(out.starts_with(crate::tools::IMAGE_MARKER), "{out}");
        let msg = crate::tools::image_followup(&ctx, "view_image", &out).expect("picture turn");
        assert_eq!(msg.role, "user");
        let imgs = msg.images.unwrap();
        assert!(imgs[0].starts_with("data:image/png;base64,"), "{}", imgs[0]);
        // plain results never grow a picture turn, and a missing file is not an error
        assert!(crate::tools::image_followup(&ctx, "shell", "ls output").is_none());
        assert!(crate::tools::image_followup(&ctx, "view_image", "[[image:nope.png]] x").is_none());
        assert!(crate::tools::view_image(&ctx, "notes.txt").is_err(), "non-image extensions refused");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn schema_hint_answers_about_the_table_the_query_named() {
        // The hint used to dump every table. workspace.db's 13 near-identical
        // graduation_watch_* clones sort before `positions`, so the one line the
        // agent needed was pushed past the point the reply got read — 132
        // identical failures in two hours, every one of them this error.
        let cfg: crate::config::Config = toml::from_str(
            "ceo_model = \"p/m\"\n[[providers]]\nname = \"p\"\nbase_url = \"http://x\"\napi_key_env = \"X\"\npaid_models = [\"m\"]\n",
        )
        .unwrap();
        let root = std::env::temp_dir().join("khan-sql-scope-test");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let ctx = crate::tools::ToolCtx {
            cfg,
            store: std::sync::Arc::new(crate::state::Store::open(":memory:").unwrap()),
            workspace: root.clone(),
            http: reqwest::Client::new(),
            http_proxy: None,
        };
        crate::tools::sql::run(&ctx, "CREATE TABLE positions(id INTEGER, asset TEXT, note TEXT)").unwrap();
        for t in ["graduation_watch_a", "graduation_watch_b", "graduation_watch_c"] {
            crate::tools::sql::run(&ctx, &format!("CREATE TABLE {t}(ts TEXT, mint TEXT)")).unwrap();
        }
        let msg = format!("{:#}", crate::tools::sql::run(&ctx, "SELECT mint FROM positions").unwrap_err());
        assert!(msg.contains("positions(id, asset, note)"), "names the asked-about table: {msg}");
        assert!(!msg.contains("graduation_watch_a("), "does not dump unrelated tables: {msg}");

        // A wrong TABLE name matches nothing, so the useful answer is the list
        // of names that do exist rather than every column in the database.
        let msg = format!("{:#}", crate::tools::sql::run(&ctx, "SELECT * FROM run_log").unwrap_err());
        assert!(msg.contains("tables: "), "falls back to the name list: {msg}");
        assert!(msg.contains("positions"), "{msg}");
        assert!(!msg.contains("positions(id"), "no column dump on the fallback: {msg}");

        // INSERT phrases a bad column differently and used to get no hint at all.
        let msg = format!("{:#}", crate::tools::sql::run(&ctx, "INSERT INTO positions(asset, amount) VALUES ('SOL', 1)").unwrap_err());
        assert!(msg.contains("has no column named"), "original error kept: {msg}");
        assert!(msg.contains("positions(id, asset, note)"), "insert gets the same hint: {msg}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn config_refuses_to_deny_its_own_floor_seat() {
        // ceo_model is both the re-home target and what hire is told to pick;
        // denying it would make both loop. The load must fail, not limp.
        let dir = std::env::temp_dir().join("khan-denied-floor-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("khan.toml");
        std::fs::write(
            &path,
            "ceo_model = \"p/glm5\"\nseat_denylist = [\"glm5\"]\n[[providers]]\nname = \"p\"\nbase_url = \"http://x\"\napi_key_env = \"KHAN_TEST_NO_SUCH_KEY\"\npaid_models = [\"glm5\"]\n",
        )
        .unwrap();
        let err = crate::config::Config::load(path.to_str().unwrap()).err().expect("load must fail");
        assert!(format!("{err:#}").contains("seat_denylist"), "{err:#}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_denied_seat_is_refused_at_hire_and_never_matches_its_successor() {
        // The model policy has said "deepseek is never a seat, re-home at next
        // dispatch" since 2026-08-30 with nothing enforcing it; on 2026-09-01
        // four agents were still sitting on deepseek and superseded glm5.
        let cfg: crate::config::Config = toml::from_str(
            "ceo_model = \"p/glm53flash\"\nseat_denylist = [\"deepseekv4flash\", \"glm5\"]\n\
             [[providers]]\nname = \"p\"\nbase_url = \"http://x\"\napi_key_env = \"X\"\n\
             paid_models = [\"glm53flash\", \"deepseekv4flash\", \"glm5\"]\n",
        )
        .unwrap();
        assert!(cfg.seat_denied("p/deepseekv4flash"), "full form denied");
        assert!(cfg.seat_denied("deepseekv4flash"), "bare slug denied");
        assert!(cfg.seat_denied("P/GLM5"), "case-insensitive");
        // The whole point of matching the slug exactly: glm53flash is the seat
        // the company is supposed to be ON, and it starts with the denied "glm5".
        assert!(!cfg.seat_denied("p/glm53flash"), "successor must not be caught by prefix");
        assert!(!cfg.seat_denied("p/deepseekv4flash0731"), "a different slug is a different model");
        // An empty denylist is the default and must deny nothing.
        let open: crate::config::Config = toml::from_str(
            "ceo_model = \"p/m\"\n[[providers]]\nname = \"p\"\nbase_url = \"http://x\"\napi_key_env = \"X\"\npaid_models = [\"m\"]\n",
        )
        .unwrap();
        assert!(!open.seat_denied("p/deepseekv4flash"), "unset denylist denies nothing");
    }

    #[test]
    fn dispatch_classes_and_shapes_read_the_day_the_way_the_founder_did() {
        use crate::agent::{classify_task, task_shape};
        assert_eq!(classify_task("Verify-what-landed on the Pons lane, objective:39"), "check");
        assert_eq!(classify_task("SIZING GATE RE-RUN (read-only) post restart"), "check");
        assert_eq!(classify_task("Spot-check scan-mgr's on-chain claims wallet balance"), "check");
        assert_eq!(classify_task("Build the launch kit for PINKPROOF: art, metadata, dry-run"), "build");
        assert_eq!(classify_task("Write ONE new zero-model-cost routine script"), "build");
        assert_eq!(classify_task("Execute the funding leg: bridge 0.05 SOL"), "build");
        // the verb that comes first wins: "build X, then verify" is a build
        assert_eq!(classify_task("Ship the page, then verify it byte-identical"), "build");
        assert_eq!(classify_task("Please look into the market"), "other");
        // four differently-worded PINKPROOF dispatches collapse to one shape
        let a = task_shape(Some(5), "METADATA GATE COIN IMAGE for the PINKPROOF launch (CEO override in force)");
        let b = task_shape(Some(5), "metadata gate: coin image for the pinkproof launch — retry");
        assert_eq!(a, b, "{a} vs {b}");
        assert_ne!(task_shape(Some(5), "x"), task_shape(Some(6), "x"), "objective is part of the shape");
    }

    #[test]
    fn the_fourth_repeat_and_the_fourth_consecutive_check_are_refused() {
        use crate::agent::admit_dispatch;
        let store = crate::state::Store::open(":memory:").unwrap();
        // three identical shapes pass, the fourth is routine work
        for _ in 0..3 {
            assert!(admit_dispatch(&store, "builder", Some(5), "Generate the METADATA GATE coin image for PINKPROOF").is_none());
        }
        let why = admit_dispatch(&store, "builder", Some(5), "generate the metadata gate coin image for pinkproof, again").unwrap();
        assert!(why.contains("ROUTINE") && why.contains("4th"), "{why}");
        // checks: three in a row on #39 pass, the fourth is refused, a build resets
        for t in ["Verify the relay quote", "Recheck the deployer balance", "Audit the evidence file"] {
            assert!(admit_dispatch(&store, "ops", Some(39), t).is_none(), "{t}");
        }
        let why = admit_dispatch(&store, "ops", Some(39), "Confirm the oracle reading").unwrap();
        assert!(why.contains("#39") && why.contains("BUILDS"), "{why}");
        assert!(admit_dispatch(&store, "pons-mgr", Some(39), "Execute the funding leg").is_none());
        assert!(admit_dispatch(&store, "ops", Some(39), "Verify the funding landed").is_none(), "one check after a build is fine");
        // upkeep (objective None or 0) never trips the consecutive-check budget
        for t in ["Verify a", "Verify b", "Verify c", "Verify d"] {
            assert!(admit_dispatch(&store, "ops", None, t).is_none());
        }
        for t in ["Verify e", "Verify f", "Verify g", "Verify h"] {
            assert!(admit_dispatch(&store, "ops", Some(0), t).is_none(), "{t}");
        }
        // ...unless the task names the objective it is really for
        for t in ["obj34 cycle 18 floor sweep", "QA gate on obj 9 draft", "Objective #18 M1-M6 bookkeeping only"] {
            let why = admit_dispatch(&store, "ops", Some(0), t).unwrap();
            assert!(why.contains("objective="), "{t}: {why}");
        }
        assert_eq!(crate::agent::named_objective("obj 35 / obj 39 follow-through"), Some(35));
        assert_eq!(crate::agent::named_objective("objective=46 — audit; escalated from obj 9"), Some(46));
        assert_eq!(crate::agent::named_objective("obj#5 trend-scout, door 1 of 3"), Some(5));
        // a manager's delegate carries no objective field: the one the task names is its tag
        assert!(admit_dispatch(&store, "scout", crate::agent::named_objective("obj#5 trend-scout, door 1 of 3"), "obj#5 trend-scout, door 1 of 3").is_none());
        assert_eq!(crate::agent::named_objective("read-only contract recon, no objective yet"), None);
        assert!(admit_dispatch(&store, "ops", Some(0), "Kill the objectionable log noise").is_none());
        // the CEO cannot block its episode on a manager's whole crew; a manager
        // may still run its own workers inline
        store.save_agent("pons-mgr", "pons lane", "pons-mgr", "m", "[]");
        store.set_manager("pons-mgr", true);
        store.save_agent("chainwatch-1", "reads chains", "chainwatch-1", "m", "[]");
        let why = crate::agent::blocking_manager_run(&store, "CEO", "pons-mgr").unwrap();
        assert!(why.contains("dispatch(pons-mgr"), "{why}");
        assert!(crate::agent::blocking_manager_run(&store, "CEO", "chainwatch-1").is_none());
        assert!(crate::agent::blocking_manager_run(&store, "pons-mgr", "chainwatch-1").is_none());
        // the board carries the mix and flags the all-checks objective
        store.add_objective("pons", 2);
        let mix = store.objective_mix_24h();
        assert_eq!(mix.get(&39), Some(&(1, 4)));
        assert_eq!(mix.get(&5), Some(&(3, 0)));
    }

    #[test]
    fn an_explore_lane_only_builds_when_it_names_the_idea_it_advances() {
        use crate::agent::{admit_dispatch, names_revenue_idea};
        let store = crate::state::Store::open(":memory:").unwrap();
        let scan = store.add_objective("opportunity scan", 4);
        assert!(store.set_objective_kind(scan, "explore"));
        let lane = store.add_objective("trend launch", 1);
        assert!(store.set_objective_kind(lane, "profit"));
        // another scan cycle is generation, however it is worded
        for t in ["Run the opportunity scan cycle 31", "Build the cycle-32 beat map"] {
            assert!(admit_dispatch(&store, "scan-mgr", Some(scan), t).is_none());
        }
        assert_eq!(store.objective_mix_24h().get(&scan), Some(&(0, 0)), "generation is not a build");
        // naming the row it moves is
        assert!(admit_dispatch(&store, "scan-mgr", Some(scan), "Build id65 stage 2 into a lane").is_none());
        assert_eq!(store.objective_mix_24h().get(&scan), Some(&(1, 0)));
        // the reclassification is explore-only: an execution lane still builds
        assert!(admit_dispatch(&store, "launch-mgr", Some(lane), "Build the next costume").is_none());
        assert_eq!(store.objective_mix_24h().get(&lane), Some(&(1, 0)));
        for t in ["id65 stage 2", "advance row 54", "idea 17 gate", "obj35 id 9 promote"] {
            assert!(names_revenue_idea(t), "{t}");
        }
        for t in ["objective #35 cycle 31", "did the grid render", "no ideas yet"] {
            assert!(!names_revenue_idea(t), "{t}");
        }
    }

    #[test]
    fn a_seat_that_keeps_stalling_is_benched_even_when_it_is_the_first_rung() {
        use crate::agent::{is_stall, stall_strike, STALL_STRIKES, STALL_WINDOW};
        // the ~128s cuts of 2026-09-02: the gateway relays the upstream status
        let cut = r#"upstream timed out mid-generation: {"error":{"message":"a source refused this request (timeout, upstream status 524)"}}"#;
        assert!(is_stall(cut, 128));
        // a slow failure counts whatever it says — the wait is the damage
        assert!(is_stall("502 Bad Gateway: a source refused this request", 135));
        // a fast failure is a fault to diagnose, not a stalled route
        assert!(!is_stall("429 rate limited", 2));
        // a speed-floor refusal is a stall verdict delivered early
        assert!(is_stall("bu0y/glm53flash: no route meets the speed floor right now — {\"error\":{\"type\":\"unmet_speed\"}}", 3));
        assert!(!is_stall("400 bad body", 1));

        let now = std::time::Instant::now();
        let mut times: Vec<std::time::Instant> = Vec::new();
        assert_eq!(stall_strike(&mut times, now), 1);
        assert_eq!(stall_strike(&mut times, now), 2);
        assert_eq!(stall_strike(&mut times, now), STALL_STRIKES, "the third inside the window benches it");
        // strikes age out: two old ones plus a fresh one is not a bench
        let stale = now - STALL_WINDOW - std::time::Duration::from_secs(1);
        let mut aged = vec![stale, stale];
        assert_eq!(stall_strike(&mut aged, now), 1);
    }

    #[test]
    fn a_seat_moves_to_the_peer_the_company_actually_pays_less_for() {
        use crate::llm::Usage;
        let cfg: crate::config::Config = toml::from_str(include_str!("../khan.toml.example")).unwrap();
        assert_eq!(cfg.peers_of("bu0y/glm53flash"), vec!["bu0y/gpt56luna".to_string()]);
        assert_eq!(cfg.peers_of("bu0y/gpt56luna"), vec!["bu0y/glm53flash".to_string()]);
        assert!(cfg.peers_of("bu0y/grok46").is_empty());
        assert_eq!(cfg.peer_switch_pct, 25);

        let store = crate::state::Store::open(":memory:").unwrap();
        let fill = |tokens: u64, micros: u64| Usage { prompt_tokens: tokens, completion_tokens: 0, billed_micros: micros };
        // four fills are not a price yet
        for _ in 0..4 {
            store.record_model_call("bu0y/glm53flash", 1000, true, "", fill(100_000, 20_000));
        }
        assert_eq!(store.realized_price("bu0y/glm53flash", 3), None);
        // the fifth makes it one: 100k micro$ over 500k tokens = 200k per 1M
        store.record_model_call("bu0y/glm53flash", 1000, true, "", fill(100_000, 20_000));
        assert_eq!(store.realized_price("bu0y/glm53flash", 3), Some(200_000));
        // a minimum-charge fill and a failed call say nothing about the rate
        store.record_model_call("bu0y/glm53flash", 1000, true, "", fill(20, 2_000));
        store.record_model_call("bu0y/glm53flash", 1000, false, "boom", Usage::default());
        assert_eq!(store.realized_price("bu0y/glm53flash", 3), Some(200_000));
        // a peer with no fills has no price, so there is nothing to move to
        assert_eq!(store.realized_price("bu0y/gpt56luna", 3), None);
        // a seat that keeps refusing is not one to move onto: five calls, one answered
        assert_eq!(store.success_rate("bu0y/gpt56luna", 3), None);
        for i in 0..5 {
            store.record_model_call("bu0y/gpt56luna", 100, i == 0, if i == 0 { "" } else { "503 below_floor" }, Usage::default());
        }
        assert_eq!(store.success_rate("bu0y/gpt56luna", 3), Some((1, 5)));
        assert!(1 * 100 < 5 * crate::agent::PEER_MIN_OK_PCT);
        // a screenshot in the last day pins an agent to its home seat
        store.kv_set("vision_agent:site-mgr", &chrono::Utc::now().to_rfc3339());
        store.kv_set("vision_agent:old-hand", &(chrono::Utc::now() - chrono::Duration::hours(30)).to_rfc3339());
        let fresh = |n: &str| {
            store
                .kv_get(&format!("vision_agent:{n}"))
                .and_then(|ts| chrono::DateTime::parse_from_rfc3339(&ts).ok())
                .is_some_and(|t| (chrono::Utc::now() - t.with_timezone(&chrono::Utc)).num_hours() < 24)
        };
        assert!(fresh("site-mgr"));
        assert!(!fresh("old-hand"));
        assert!(!fresh("never-looked"));
        // the shipped caps ride luna and nothing else
        let luna = cfg.model_caps.get("bu0y/gpt56luna").unwrap();
        assert_eq!((luna.max_input_per_1m, luna.max_output_per_1m), (Some(20_000), Some(120_000)));
        assert!(cfg.model_caps.get("bu0y/glm53flash").is_none());
    }

    #[test]
    fn a_speed_floor_rides_only_the_provider_that_set_it_and_unmet_speed_is_not_a_wait() {
        use crate::llm::unmet_speed;
        // the shipped config carries the floor on bu0y and nowhere else
        let cfg: crate::config::Config = toml::from_str(include_str!("../khan.toml.example")).unwrap();
        let bu0y = cfg.providers.iter().find(|p| p.name == "bu0y").unwrap();
        assert_eq!(bu0y.min_tokens_per_sec, Some(12));
        for p in cfg.providers.iter().filter(|p| p.name != "bu0y") {
            assert_eq!(p.min_tokens_per_sec, None, "{}", p.name);
        }
        // the refusal is recognised by type, not by its changing message
        assert!(unmet_speed(r#"{"error":{"message":"fastest recent route decodes at 9.8 tokens/s","type":"unmet_speed"}}"#));
        assert!(!unmet_speed(r#"{"error":{"message":"below floor","type":"api_error","retry_max_tokens":4096}}"#));
        assert!(!unmet_speed("not json"));
    }

    #[test]
    fn a_named_smaller_ceiling_is_a_retry_not_a_dead_request() {
        use crate::llm::retry_max_tokens;
        // the 400 that is really "ask for less" — the number rides the body
        let four_hundred = r#"{"error":{"message":"max_tokens exceeds what this model can produce inside the fill ceiling","type":"invalid_request_error","retry_max_tokens":8192}}"#;
        assert_eq!(retry_max_tokens(four_hundred), Some(8192));
        // the 503 below_floor has carried it all along
        assert_eq!(
            retry_max_tokens(r#"{"error":{"message":"below floor","type":"api_error","retry_max_tokens":4096}}"#),
            Some(4096)
        );
        // a plain refusal names no ceiling and must stay a failure
        for body in [
            r#"{"error":{"message":"bad body","type":"invalid_request_error"}}"#,
            r#"{"error":{"message":"nope","retry_max_tokens":0}}"#,
            "not json at all",
            "",
        ] {
            assert_eq!(retry_max_tokens(body), None, "{body}");
        }
        // never above the binary's own ceiling, whatever the gateway says
        assert_eq!(retry_max_tokens(r#"{"error":{"retry_max_tokens":999999999}}"#), Some(65_536));

        // A budget WE chose and blew is the model's own doing, and no other
        // model would fare differently — the caller must not walk the ladder.
        // A budget the GATEWAY shrank to this route's recent speed is a routing
        // problem: another model is quoted its own ceiling, so the ladder helps.
        let ours = anyhow::Error::new(crate::llm::Truncated {
            max_tokens: 65_536, reasoning_tokens: 65_536, gateway_capped: false,
        });
        let theirs = anyhow::Error::new(crate::llm::Truncated {
            max_tokens: 6_400, reasoning_tokens: 6_400, gateway_capped: true,
        });
        assert!(crate::llm::truncation(&ours).is_some_and(|t| !t.gateway_capped));
        assert!(crate::llm::truncation(&theirs).is_some_and(|t| t.gateway_capped));
        assert!(theirs.to_string().contains("the ceiling the gateway said would fit"), "{theirs}");
        assert!(!ours.to_string().contains("gateway"), "{ours}");
        // a summary never asks for the model's whole ceiling
        assert!(crate::agent::SUMMARY_MAX_TOKENS < 65_536 / 4);
    }

    #[test]
    fn a_tweet_gets_one_reply_and_the_memory_outlives_the_day() {
        let store = crate::state::Store::open(":memory:").unwrap();
        assert_eq!(store.x_reply_to("2094781874174357640"), None);
        store.x_record_reply("2094781874174357640", "2094800000000000001");
        let (_, ours) = store.x_reply_to("2094781874174357640").unwrap();
        assert_eq!(ours, "2094800000000000001");
        // the billing ledger rolls at UTC midnight; the reply memory must not
        store.x_mark_seen(&["2094781874174357640"], "2026-09-01");
        store.x_mark_seen(&["2094913224747737227"], "2026-09-02");
        assert!(store.x_reply_to("2094781874174357640").is_some(), "a new day is not a new conversation");
        // a different tweet in the same thread is still answerable
        assert_eq!(store.x_reply_to("2094913224747737227"), None);
        // recording twice keeps the first reply as the record
        store.x_record_reply("2094781874174357640", "2094899999999999999");
        assert_eq!(store.x_reply_to("2094781874174357640").unwrap().1, "2094800000000000001");
    }

    #[test]
    fn ideas_past_their_own_review_date_stand_in_the_brief() {
        let dir = std::env::temp_dir().join("khan-overdue-ideas-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // no workspace.db at all: no line, and the read must not create one
        assert_eq!(crate::agent::overdue_ideas_line(&dir), "");
        assert!(!dir.join("workspace.db").exists());
        let c = rusqlite::Connection::open(dir.join("workspace.db")).unwrap();
        c.execute_batch(
            "CREATE TABLE revenue_ideas(id INTEGER PRIMARY KEY, name TEXT, status TEXT, review_date TEXT);
             INSERT INTO revenue_ideas VALUES
               (9,'USDC yield','candidate','2026-08-29'),
               (16,'EQUITYCAT rider','candidate','2026-09-01'),
               (44,'memorial costume','premise','2099-01-01'),
               (60,'tape-death exit','parked','2026-08-01'),
               (12,'LP fee review','done','2026-08-01'),
               (7,'no date','premise','');",
        )
        .unwrap();
        let due = crate::tools::sql::overdue_ideas(&dir, "2026-09-02");
        let ids: Vec<i64> = due.iter().map(|(id, ..)| *id).collect();
        assert_eq!(ids, vec![9, 16], "only undecided rows whose date has passed, oldest first");
        let line = crate::agent::overdue_ideas_line(&dir);
        assert!(line.contains("id9 USDC yield") && line.contains("2026-08-29"), "{line}");
        assert!(line.contains("kill it with the number"), "{line}");
    }

    #[test]
    fn quiet_heartbeats_back_off_and_events_reset() {
        use crate::agent::backoff_interval;
        assert_eq!(backoff_interval(300, 1800, 0), 300);
        assert_eq!(backoff_interval(300, 1800, 1), 600);
        assert_eq!(backoff_interval(300, 1800, 2), 1200);
        assert_eq!(backoff_interval(300, 1800, 3), 1800, "capped");
        assert_eq!(backoff_interval(300, 1800, 40), 1800, "stays capped");
        assert_eq!(backoff_interval(300, 0, 5), 300, "0 max means no backoff");
        assert_eq!(backoff_interval(300, 100, 1), 300, "a ceiling below the base is the base");
    }

    #[test]
    fn fuel_sends_are_refused_above_the_refill_target_and_only_to_the_deposit() {
        use crate::tools::fuel_send_blocked;
        let store = crate::state::Store::open(":memory:").unwrap();
        let dep = "GfW6tV82eS6iY1LAC885Pmr9Hfchdj3m9hmohieXsCBR";
        let send = format!("{{\"to\":\"{dep}\",\"amount\":14}}");
        // no poll yet: nothing is known, nothing is blocked
        assert!(fuel_send_blocked(&store, &send).is_none());
        store.kv_set("fuel_deposit_body", &format!("{{\"address\":\"{dep}\",\"chain\":\"solana\"}}"));
        store.kv_set("fuel_refill_target_micros", "60000000"); // $60
        // $117 in the tank: the 2026-09-01 case — refused, and the reason names the numbers
        store.kv_set("fuel_available_micros", "117000000");
        let why = fuel_send_blocked(&store, &send).expect("must refuse");
        assert!(why.contains("$117.00") && why.contains("$60.00"), "{why}");
        // the same address on a raw shell line is caught too
        assert!(fuel_send_blocked(&store, &format!("spl-token transfer USDC 14 {dep}")).is_some());
        // a send anywhere else is none of this gate's business
        assert!(fuel_send_blocked(&store, "{\"to\":\"9gsVSHrcrqtqiaKn4oT4t4vKqVmVboBpVnm5VrYsk3aV\",\"amount\":14}").is_none());
        // at or below the target the kernel's alert has fired: the send is allowed
        store.kv_set("fuel_available_micros", "60000000");
        assert!(fuel_send_blocked(&store, &send).is_none());
        store.kv_set("fuel_available_micros", "8000000");
        assert!(fuel_send_blocked(&store, &send).is_none());
        // the brief states the rule once a reading exists, and is silent before
        assert!(crate::agent::fuel_brief_line(&crate::state::Store::open(":memory:").unwrap()).is_empty());
        let line = crate::agent::fuel_brief_line(&store);
        assert!(line.contains("$8.00") && line.contains("REFUSED"), "{line}");
    }

    #[test]
    fn ticker_rows_age_out_and_real_events_do_not() {
        // The stats daemon writes ~80KB into run_log every 12s; /data hit 100%
        // on 2026-09-01 22:53Z. Only the latest rows are read, so the window
        // is enforced by the binary, not by a routine staying registered.
        let store = crate::state::Store::open(":memory:").unwrap();
        let old = (chrono::Utc::now() - chrono::Duration::hours(7)).to_rfc3339();
        let daemon_style = (chrono::Utc::now() - chrono::Duration::hours(7)).format("%Y-%m-%dT%H:%M:%S.000000+00:00").to_string();
        store.raw_log_at(&old, "khan", "stats", "{}");
        store.raw_log_at(&daemon_style, "khan", "stats", "{}");
        store.raw_log_at(&old, "core", "team", "{}");
        store.raw_log_at(&old, "CEO", "dispatch", "{}"); // a real event, same age
        store.log("khan", "stats", "{}"); // fresh
        assert_eq!(store.prune_ticker(), 3, "the three aged ticker rows go");
        let left: Vec<(String, String)> = store.log_events_for_test();
        assert!(left.iter().any(|(e, _)| e == "dispatch"), "real events are never pruned by age here: {left:?}");
        assert_eq!(left.iter().filter(|(e, _)| e == "stats").count(), 1, "the fresh ticker row stays: {left:?}");
        assert_eq!(store.prune_ticker(), 0, "idempotent");
    }

    #[test]
    fn founder_directives_stand_in_the_brief_until_acked() {
        // The 23:44Z x_api_ops fold request was read, stated as "must land in
        // the skill", then lost at the episode cut-off. A khan tell now stays
        // in the brief until the CEO acks it; a Telegram turn does not.
        let store = crate::state::Store::open(":memory:").unwrap();
        store.queue_message("fold the reply rules into x_api_ops");
        store.queue_message("[via Telegram] hey, how's it going");
        assert!(store.open_directives().is_empty(), "undelivered messages are not yet open");
        let drained = store.drain_messages();
        assert_eq!(drained.len(), 2);
        let id = drained[0].0;
        let open = store.open_directives();
        assert_eq!(open.len(), 1, "telegram chat is not a standing directive: {open:?}");
        assert_eq!(open[0].0, id);
        let text = crate::agent::open_directives_text(&open);
        assert!(text.contains("OPEN FOUNDER DIRECTIVES"), "{text}");
        assert!(text.contains(&format!("#{id} [")), "{text}");
        assert!(text.contains("fold the reply rules"), "{text}");
        // draining again does not re-deliver, and the directive still stands
        assert!(store.drain_messages().is_empty());
        assert_eq!(store.open_directives().len(), 1);
        assert!(!store.ack_message(999), "unknown id is not an ack");
        assert!(store.ack_message(id));
        assert!(!store.ack_message(id), "an ack is once");
        assert!(store.open_directives().is_empty());
        assert!(crate::agent::open_directives_text(&[]).is_empty(), "no block when nothing is open");
    }

    #[test]
    fn dm_bait_is_flagged_and_real_questions_are_not() {
        use crate::tools::x::looks_like_bait;
        for bait in [
            "DM me Let's pump it 💪",
            "Can we talk privately?",
            "Dm me now  📥",
            "check my bio for the alpha",
            "send me a DM and I'll explain",
        ] {
            assert!(looks_like_bait(bait), "should flag: {bait}");
        }
        // the reply that earned an answer tonight, and the kind of question
        // the open-source pointer is for — neither is bait
        for real in [
            "@KHAN_AI_SOL What're you building here?",
            "how does the kill clock decide?",
            "is the code public? where's the repo",
            "what model runs the ceo",
        ] {
            assert!(!looks_like_bait(real), "must not flag: {real}");
        }
    }

    #[test]
    fn stream_subscribes_to_replies_and_quotes_not_just_mentions() {
        // post.mention.create does not fire for the implicit mention a reply
        // carries, so for two days replies to our posts never pushed and sat
        // until a paid poll found them. Every engagement event must be wanted,
        // and only the ones actually missing get created.
        use crate::tools::x::{missing_subscriptions, ENGAGEMENT_EVENTS};
        assert!(ENGAGEMENT_EVENTS.contains(&"post.reply.create"), "replies are the bulk of engagement");
        assert!(ENGAGEMENT_EVENTS.contains(&"post.quote.create"));
        assert!(ENGAGEMENT_EVENTS.contains(&"post.mention.create"));
        let me = "2094212160943476736";
        // the live state on 2026-09-01: mentions only
        let list = serde_json::json!({"data": [
            {"event_type": "post.mention.create", "filter": {"user_id": me}},
            {"event_type": "post.reply.create", "filter": {"user_id": "someone-else"}},
        ]});
        let missing = missing_subscriptions(&list, me);
        assert_eq!(missing, vec!["post.reply.create", "post.quote.create"], "{missing:?}");
        // nothing to do when all three exist for us
        let full = serde_json::json!({"data": ENGAGEMENT_EVENTS.iter()
            .map(|e| serde_json::json!({"event_type": e, "filter": {"user_id": me}})).collect::<Vec<_>>()});
        assert!(missing_subscriptions(&full, me).is_empty());
        // an empty or malformed list means create everything, never skip
        assert_eq!(missing_subscriptions(&serde_json::json!({}), me).len(), ENGAGEMENT_EVENTS.len());
    }

    #[test]
    fn screenshot_output_path_is_workspace_relative_not_joined() {
        // The browser child runs with the workspace as its cwd, so a joined
        // path doubles the segment: every screenshot landed in
        // workspace/workspace/... while the byte check read the placeholder at
        // the real path and reported "produced no bytes" for every URL.
        let cmd = crate::tools::web::render_cmd("https://fart.dev", "shot", "pumpfun/tmp/a.png");
        assert!(cmd.ends_with("'pumpfun/tmp/a.png'"), "{cmd}");
        assert!(!cmd.contains("workspace/"), "output path must not carry a workspace segment: {cmd}");
        assert!(cmd.contains("'https://fart.dev' shot "), "{cmd}");
        // a quote in the url cannot break out of its shell word
        let cmd = crate::tools::web::render_cmd("https://x.test/'; rm -rf /;'", "text", "");
        assert!(!cmd.contains("rm -rf /;'"), "{cmd}");
        assert!(cmd.contains("%27"), "{cmd}");
    }

    #[test]
    fn idle_capacity_line_names_the_waste() {
        // "Everything is owned" closed episodes while 51% of 2026-09-01 ran at
        // zero-or-one active agents against ten open objectives — the line
        // turns that into a number the CEO must answer before finish_episode.
        let line = crate::agent::idle_capacity_line(10, 1, 14);
        assert!(line.contains("10 objectives active"), "{line}");
        assert!(line.contains("1 of 14"), "{line}");
        assert!(line.contains("13 idle"), "{line}");
        assert!(line.contains("finish_episode"), "{line}");
        assert!(line.contains("why waiting beats working"), "{line}");
    }

    #[test]
    fn fuel_anchor_outranks_stale_payload_errors() {
        // With a gauge reading: the authoritative dollar figure leads, and the
        // stale-error warning is present (the 08-31 false emergency: a 41h-old
        // 402 in the raw usage payload read as a live $0.038 tank).
        let line = crate::agent::fuel_anchor(Some((
            52_690_988,
            std::time::Instant::now(),
            916_000.0, // micro$/hr EMA -> ~$21.98/day
        )));
        assert!(line.contains("$52.69 available"), "{line}");
        assert!(line.contains("authoritative"), "{line}");
        assert!(line.contains("days old"), "{line}");
        // Without a poll yet: no invented number, but the caution still stands
        // and names the canonical verification path.
        let cold = crate::agent::fuel_anchor(None);
        assert!(!cold.contains('$') || !cold.contains("available,"), "{cold}");
        assert!(cold.contains("GET /account"), "{cold}");
    }

    #[test]
    fn sql_tool_description_carries_the_live_table_list() {
        let dir = std::env::temp_dir().join(format!("khan-sqlhint-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // No workspace.db yet: the description stays untouched (and no db file
        // is created as a side effect).
        let mut schemas = crate::tools::work_schemas();
        let before = schemas.iter().find(|t| t["function"]["name"] == "sql").unwrap()["function"]["description"].as_str().unwrap().to_string();
        crate::tools::hint_sql_tables(&dir, &mut schemas);
        assert!(!dir.join("workspace.db").exists(), "read-only open must not create the db");
        assert_eq!(schemas.iter().find(|t| t["function"]["name"] == "sql").unwrap()["function"]["description"].as_str().unwrap(), before);
        // With tables present, their names land in the sql tool's description —
        // agents see what exists before writing the query, not after a miss.
        let conn = rusqlite::Connection::open(dir.join("workspace.db")).unwrap();
        conn.execute_batch("CREATE TABLE closed_positions(mint, asset); CREATE TABLE revenue_ideas(id, name);").unwrap();
        drop(conn);
        crate::tools::hint_sql_tables(&dir, &mut schemas);
        let desc = schemas.iter().find(|t| t["function"]["name"] == "sql").unwrap()["function"]["description"].as_str().unwrap().to_string();
        assert!(desc.contains("closed_positions") && desc.contains("revenue_ideas"), "table names in description: {desc}");
        assert!(desc.starts_with(&before), "hint appends, never replaces");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn x_post_cost_applies_the_url_surcharge() {
        use crate::tools::x::post_cost;
        assert_eq!(post_cost("shipped a new build today"), 0.015);
        for t in ["see https://khanbot.fun/log", "http://a.b", "at www.example.com now"] {
            assert_eq!(post_cost(t), 0.200, "{t} should bill the URL rate");
        }
    }

    #[test]
    fn usdc_delta_reads_only_our_address_and_mint() {
        use crate::tools::x::usdc_delta;
        let mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
        let tx = serde_json::json!({"result": {"meta": {
            "err": null,
            "preTokenBalances": [
                {"mint": mint, "owner": "FundAddr", "uiTokenAmount": {"uiAmount": 10.0}},
                {"mint": mint, "owner": "SenderAddr", "uiTokenAmount": {"uiAmount": 50.0}}
            ],
            "postTokenBalances": [
                {"mint": mint, "owner": "FundAddr", "uiTokenAmount": {"uiAmount": 15.0}},
                {"mint": mint, "owner": "SenderAddr", "uiTokenAmount": {"uiAmount": 45.0}},
                {"mint": "SomeOtherMint", "owner": "FundAddr", "uiTokenAmount": {"uiAmount": 999.0}}
            ]
        }}});
        assert!((usdc_delta(&tx, "FundAddr") - 5.0).abs() < 1e-9, "credits the USDC that arrived");
        assert!(usdc_delta(&tx, "SenderAddr") < 0.0, "the sender's delta is negative, never creditable");
        // A fund account that did not exist before the transfer has no pre row.
        let fresh = serde_json::json!({"result": {"meta": {
            "preTokenBalances": [],
            "postTokenBalances": [{"mint": mint, "owner": "FundAddr", "uiTokenAmount": {"uiAmount": 2.5}}]
        }}});
        assert!((usdc_delta(&fresh, "FundAddr") - 2.5).abs() < 1e-9);
    }

    #[tokio::test]
    async fn solana_rpc_request_shape_probe() {
        // Probes the getTransaction wire shape against the public mainnet RPC
        // (no key needed). A signature that cannot exist must come back as a
        // JSON-RPC result of null — proving the request shape is accepted.
        // Self-skips when offline so the suite stays runnable anywhere.
        let body = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "getTransaction",
            "params": ["1".repeat(64), {"encoding": "jsonParsed", "maxSupportedTransactionVersion": 0}]
        });
        let resp = match reqwest::Client::new()
            .post("https://api.mainnet-beta.solana.com")
            .json(&body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(_) => {
                eprintln!("solana_rpc_request_shape_probe: skipped (offline)");
                return;
            }
        };
        let v: serde_json::Value = serde_json::from_str(&resp.text().await.unwrap_or_default()).expect("rpc response not json");
        assert!(v["result"].is_null() && v["error"].is_null(), "rpc rejected the request shape: {v}");
    }

    #[tokio::test]
    async fn live_request_shape_probe() {
        // "Probe provider-facing request shapes live before shipping", made
        // runnable: sends one real 16-token streamed request through
        // build_request and asserts the gateway accepts the wire shape.
        // Self-skips without a key so CI and keyless checkouts stay green.
        let Ok(key) = std::env::var("OPENROUTER_API_KEY") else {
            eprintln!("live_request_shape_probe: skipped (no OPENROUTER_API_KEY)");
            return;
        };
        let body = Client::build_request(
            "openai/gpt-4o-mini",
            &[Message::text("user", "Reply with the word ok.")],
            &crate::tools::work_schemas(),
            16,
        );
        let resp = reqwest::Client::new()
            .post("https://openrouter.ai/api/v1/chat/completions")
            .bearer_auth(key)
            .json(&body)
            .send()
            .await
            .expect("probe request failed to send");
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        assert!(status.is_success(), "provider rejected the request shape: {status} — {}", text.chars().take(300).collect::<String>());
        assert!(text.contains("data:"), "expected an SSE stream back, got: {}", text.chars().take(200).collect::<String>());
    }

    #[tokio::test]
    async fn shell_outcome_reports_facts_independently() {
        // exit code and timeout are separate facts — routines alert on each
        // differently, and neither should have to be parsed out of prose.
        let dir = std::env::temp_dir();
        let ok = crate::tools::shell::run_in_dir(&dir, "echo hi", Default::default()).await.unwrap();
        assert!(ok.success && !ok.timed_out);
        assert!(ok.text.contains("hi"));
        let bad = crate::tools::shell::run_in_dir(&dir, "exit 3", Default::default()).await.unwrap();
        assert!(!bad.success && !bad.timed_out);
    }

    #[test]
    fn episode_notes_roundtrip_and_roster_renders() {
        let store = crate::state::Store::open(":memory:").unwrap();
        assert!(store.last_episode_note().is_none());
        store.add_episode("2026-08-29T20:00:00Z", "founder", "did X; next: Y", 4);
        store.add_episode("2026-08-29T20:10:00Z", "report", "rated Z 5/5; phone still pending", 2);
        // The newest note is the one the next episode's brief carries.
        assert_eq!(store.last_episode_note().unwrap(), "rated Z 5/5; phone still pending");
        store.save_agent("worker-1", "role", "agent:worker-1", "bu0y/deepseekv4flash", "[]");
        store.save_agent("mgr-1", "role", "agent:mgr-1", "bu0y/glm53flash", "[]");
        store.set_manager("mgr-1", true);
        let roster = store.team_roster_text();
        assert!(roster.contains("worker-1 (bu0y/deepseekv4flash)"));
        assert!(roster.contains("mgr-1 (bu0y/glm53flash, manager)"));
        assert!(!roster.contains("CEO"));
    }

    #[test]
    fn observation_set_excludes_every_advancing_tool() {
        use crate::agent::OBSERVATION_TOOLS as OBS;
        // Advancing actions must never be classified as observation — that would
        // make the loop sleep right after real work.
        for t in ["dispatch", "delegate", "delegate_parallel", "hire", "fire", "rate_work",
                  "objectives", "update_prompt", "save_playbook", "remember", "create_skill",
                  "finish", "add_routine", "remove_routine", "write_file", "create_tool"] {
            assert!(!OBS.contains(&t), "{t} must count as advancing");
        }
        // The observed poll vectors must be in the set or the wait never engages.
        for t in ["team_status", "shell", "sql", "read_file", "web_fetch"] {
            assert!(OBS.contains(&t), "{t} must count as observation");
        }
    }

    #[test]
    fn portfolio_review_groups_by_kind_and_attributes_attention() {
        let store = crate::state::Store::open(":memory:").unwrap();
        let launches = store.add_objective("trend launches", 1);
        let social = store.add_objective("farcaster voice", 2);
        let mystery = store.add_objective("unlabeled bet", 3);
        assert!(store.set_objective_kind(launches, "profit"));
        assert!(store.set_objective_kind(social, "growth"));
        assert!(!store.set_objective_kind(mystery, "marketing"), "unknown kinds must be rejected");
        // Attention: a dispatch tags the agent to an objective; the agent's
        // thinking turns after it are attributed there.
        store.log("CEO", "dispatch", &format!("{{\"agent\":\"launch-mgr\",\"objective\":{launches},\"task\":\"scan\"}}"));
        for _ in 0..3 {
            store.log("launch-mgr", "thinking", "weighing the odds (bu0y/glm53flash)");
        }
        store.log("free-agent", "thinking", "unattributed turn");
        let review = store.portfolio_review_text("2000-01-01T00:00:00Z");
        assert!(review.contains("PROFIT LANES"), "profit group missing: {review}");
        assert!(review.contains("GROWTH / AUDIENCE"));
        // 3 of 4 thinking turns belong to the launches lane.
        assert!(review.contains("trend launches — ~75% of the company's attention"), "attribution wrong: {review}");
        assert!(review.contains("farcaster voice — ~0% of the company's attention"));
        // The unclassified lane is called out and told how to classify.
        assert!(review.contains("UNCLASSIFIED") && review.contains("unlabeled bet"));
        // A money lane without a stated numeric premise is flagged — the review
        // argues actuals-vs-premise, never streaks. Growth lanes are exempt.
        assert!(review.contains("no PREMISE line"), "unpremised profit lane must be flagged: {review}");
        assert!(!review.contains("farcaster voice —\n  ⚠"), "growth lanes are not premise-flagged");
        assert!(store.update_objective(launches, None, None, Some("PREMISE: ~1 in 15 graduates; a graduation pays ~50x the per-trial cost; trial budget 30 under floor math"), None, None));
        assert!(!store.portfolio_review_text("2000-01-01T00:00:00Z").contains("no PREMISE line"), "stated premise clears the flag");
        // A done objective leaves the review.
        assert!(store.update_objective(mystery, None, None, None, None, Some("done")));
        assert!(!store.portfolio_review_text("2000-01-01T00:00:00Z").contains("unlabeled bet"));
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
    fn repo_skills_seed_once_and_never_override() {
        // cargo test runs from the repo root, so seed() reads the real skills/
        // directory — which also guards the shipped files' format (first line
        // description, non-empty body).
        let store = crate::state::Store::open(":memory:").unwrap();
        crate::tools::skills::seed(&store);
        let names: Vec<String> = store.list_skills().into_iter().map(|(n, _)| n).collect();
        for n in [
            "palmyr_agent_infra",
            "bridge_hygiene",
            "skill_authoring",
            "web_access_from_datacenter",
            "evm_wallet_ops",
            "security_audit",
            "anti_slop_frontend",
            "pons_launchpad",
            "farcaster_ops",
            "farcaster_voice_policy",
            "jupiter_swap_v2",
            "routine_script_pattern",
            "safe_file_read_pattern",
            "pumpfun_launch",
            "pumpfun_creator_fees",
            "pumpfun_swap",
            "founder_payout",
            "wallet_anti_dusting",
            "image_generation",
            "x_api_ops",
            "trading_discipline",
            "token_vetting",
            "meme_culture",
            "no_baked_page_data",
            "page_refresh_verification",
            "vault_is_not_an_account",
            "safe_apply_to_live_tool",
            "build_in_public",
            "research_sources",
            "restart_survival",
            "sql_tool_columns",
            "launch_kill_sequence",
            "js_site_style_recon",
            "workspace_db_schema",
            "x_beat_prep",
            "token_listing_submissions",
        ] {
            assert!(names.contains(&n.to_string()), "missing seed '{n}' in {names:?}");
        }
        // A company-evolved version is never clobbered by a reseed.
        store.save_skill("bridge_hygiene", "evolved", "the company's own v2", "test").unwrap();
        crate::tools::skills::seed(&store);
        let (desc, _) = store.get_skill("bridge_hygiene").unwrap();
        assert_eq!(desc, "evolved");
        // A still-seed-origin skill DOES take a changed file as a new version:
        // fake an older seed for a real file name, reseed, expect the file text.
        store.retire_skill("evm_wallet_ops");
        store
            .save_skill("evm_wallet_ops", "old", "stale seed body", "seeded from the repo's skills/ directory")
            .unwrap();
        crate::tools::skills::seed(&store);
        let (_, content) = store.get_skill("evm_wallet_ops").unwrap();
        assert_ne!(content, "stale seed body", "changed seed file must ship as a new version");
        // Retire removes every version and the index line.
        assert!(store.retire_skill("bridge_hygiene"));
        assert!(store.get_skill("bridge_hygiene").is_none());
        // Loads land in the stats text.
        store.log_skill_load("worker-1", "evm_wallet_ops");
        assert!(store.skill_stats_text().contains("evm_wallet_ops"));
    }

    #[test]
    fn recall_surfaces_contradicting_skill_lines() {
        // The fee-premise incident: a debunk written into a skill body was
        // invisible to recall, so a scout re-derived the false premise and the
        // reviewing CEO never saw the contradiction. Recall must now search
        // skill content too.
        let store = crate::state::Store::open(":memory:").unwrap();
        store
            .save_skill(
                "growth_copy",
                "copy rules",
                "## Fee-reality facts\n- the creator-fee structure has been LIVE since 2025. \
                 There is NO upcoming fee change tied to a date. That premise is false.\n\
                 - unrelated line about tone",
                "test",
            )
            .unwrap();
        let hits = store.recall("scout found an upcoming creator-fee change premise", 5);
        let skill_hit = hits.iter().find(|h| h.starts_with("[skill growth_copy"));
        let hit = skill_hit.expect("recall must surface the skill excerpt");
        assert!(hit.contains("NO upcoming fee change"), "excerpt must carry the debunk line: {hit}");
        assert!(!hit.contains("unrelated line"), "only term-matching lines ride along");
        // A skill sharing just one common word stays out of recall (noise floor).
        store.save_skill("payouts", "payout rules", "the founder premise here is dust tests", "test").unwrap();
        let hits = store.recall("weekly premise review", 5);
        assert!(
            !hits.iter().any(|h| h.starts_with("[skill payouts")),
            "single-term overlap must not surface a skill: {hits:?}"
        );
    }

    #[test]
    fn recall_excerpt_truncation_survives_multibyte_content() {
        // 2026-08-31 crash loop: excerpt.truncate(400) landed mid-char on a
        // skill body full of em-dashes, panicking the thread and poisoning
        // the store mutex — the whole binary died on every recall after.
        let store = crate::state::Store::open(":memory:").unwrap();
        // Line long enough that the 400-byte cut falls inside a multi-byte
        // char with high probability across the repeated 3-byte em-dashes.
        let line = format!("creator fee change — {} — creator fee dates", "—".repeat(300));
        store.save_skill("fees", "fee facts", &line, "test").unwrap();
        let hits = store.recall("creator fee change dates", 5);
        assert!(hits.iter().any(|h| h.starts_with("[skill fees")), "skill must surface: {hits:?}");
    }

    #[test]
    fn telegram_chat_tail_old_and_delete_slice_correctly() {
        let store = crate::state::Store::open(":memory:").unwrap();
        for i in 1..=10 {
            let role = if i % 2 == 1 { "founder" } else { "ceo" };
            store.add_telegram_chat(role, &format!("msg {i}"));
        }
        // Tail: newest 3, oldest first.
        let tail = store.telegram_tail(3);
        assert_eq!(
            tail.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>(),
            vec!["msg 8", "msg 9", "msg 10"]
        );
        assert_eq!(store.telegram_chat_chars(), 10 * 5 + 1); // "msg N" x10, one two-digit
        // Old: everything except the newest 3, oldest first — the compaction slice.
        let old = store.telegram_old(3);
        assert_eq!(old.len(), 7);
        assert_eq!(old.first().unwrap().2, "msg 1");
        assert_eq!(old.last().unwrap().2, "msg 7");
        // Deleting through the slice leaves exactly the tail.
        store.delete_telegram_upto(old.last().unwrap().0);
        assert_eq!(store.telegram_tail(100).len(), 3);
        assert!(store.telegram_old(3).is_empty());
    }

    #[test]
    fn stale_plans_get_flagged_and_fresh_ones_do_not() {
        let store = crate::state::Store::open(":memory:").unwrap();
        let o = store.add_objective("email lane", 1);
        assert!(store.update_objective(o, None, None, Some("PIVOT TO ZOHO..."), None, None));
        // Fresh plan: no flag.
        let board = store.objectives_board(&std::collections::HashMap::new());
        assert!(!board.contains("PLAN STALE"));
        // Work advances for two days while the plan never moves: flagged.
        store.backdate_plan(o, &(chrono::Utc::now() - chrono::Duration::days(2)).to_rfc3339());
        store.touch_objective(o);
        let board = store.objectives_board(&std::collections::HashMap::new());
        assert!(board.contains("PLAN STALE? (untouched 2d"), "{board}");
        // Touching the plan clears it.
        assert!(store.update_objective(o, None, None, Some("AGENTMAIL is canonical..."), None, None));
        let board = store.objectives_board(&std::collections::HashMap::new());
        assert!(!board.contains("PLAN STALE"));
        // Legacy rows with no stamp are left unflagged rather than guessed at.
        let legacy = store.add_objective("old bet", 2);
        assert!(store.update_objective(legacy, None, None, Some("plan"), None, None));
        store.backdate_plan(legacy, "");
        assert!(!store.objectives_board(&std::collections::HashMap::new()).contains("old bet — PLAN STALE"));
    }

    #[test]
    fn objective_owners_route_render_and_revert() {
        let store = crate::state::Store::open(":memory:").unwrap();
        let o = store.add_objective("email outreach", 1);
        // No owner yet: routing lookup comes back empty.
        assert_eq!(store.objective_owner(o), None);
        assert!(store.set_objective_owner(o, "email-mgr"));
        assert_eq!(store.objective_owner(o).as_deref(), Some("email-mgr"));
        // The board shows who owns what.
        let board = store.objectives_board(&std::collections::HashMap::new());
        assert!(board.contains("owned by email-mgr"));
        // A done objective must not route anywhere.
        assert!(store.update_objective(o, None, None, None, None, Some("done")));
        assert_eq!(store.objective_owner(o), None);
        // A fired manager's objectives revert to CEO routing.
        let o2 = store.add_objective("second bet", 2);
        store.set_objective_owner(o2, "email-mgr");
        // Clears every row they owned, the done one included.
        assert_eq!(store.clear_objective_owner("email-mgr"), 2);
        assert_eq!(store.objective_owner(o2), None);
        // Empty owner clears explicitly too.
        store.set_objective_owner(o2, "other-mgr");
        assert!(store.set_objective_owner(o2, ""));
        assert_eq!(store.objective_owner(o2), None);
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

    /// Tests that set env vars and then run Config::load must not overlap:
    /// load's key-scrub deletes every secret-shaped var in the PROCESS, so a
    /// parallel test's variable can vanish between its set and its load.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn example_config_is_the_shipped_config_and_must_load() {
        let _env = ENV_LOCK.lock().unwrap();
        // The Dockerfile bakes khan.toml.example in as /app/khan.toml — a parse
        // or validation error here is a downed company, not a doc typo.
        std::env::set_var("BU0Y_API_KEY", "test-key");
        std::env::set_var("OPENROUTER_API_KEY", "test-key");
        let cfg = crate::config::Config::load("khan.toml.example").expect("khan.toml.example must load");
        assert!(!cfg.ceo_models.is_empty(), "seat ladder configured");
        assert!(cfg.ceo_max_input_price > 0 && cfg.ceo_max_output_price > 0);
    }

    #[test]
    fn routines_schedule_and_alert_flow() {
        let store = crate::state::Store::open(":memory:").unwrap();
        store.upsert_routine("claim-verify", "python3 check.py", 300, "claim rows match chain", "");
        // Never ran → due immediately; not due again right after a run.
        assert_eq!(
            store.due_routines(1000),
            vec![("claim-verify".into(), "python3 check.py".into(), "".into(), "".into(), "".into())]
        );
        // Owner assignment: alerts route to the owner; listing says who owns
        // them; clearing routes back to the CEO.
        assert!(store.set_routine_owner("claim-verify", "cfo-1"));
        assert!(!store.set_routine_owner("no-such-routine", "cfo-1"));
        assert_eq!(store.due_routines(1000)[0].4, "cfo-1");
        let listed = store.list_routines();
        assert!(listed[0].4.contains("alerts owned by cfo-1"), "{}", listed[0].4);
        assert!(store.set_routine_owner("claim-verify", ""));
        assert!(store.list_routines()[0].4.contains("alerts wake the CEO"), "{}", store.list_routines()[0].4);
        store.mark_routine_run("claim-verify", 1000, "ok");
        assert!(store.due_routines(1100).is_empty(), "not due 100s after a 300s-interval run");
        assert_eq!(store.due_routines(1300).len(), 1, "due again once the interval elapses");
        // Alerts queue for the CEO and drain exactly once.
        store.add_routine_alert("claim-verify", "ALERT: pnl row 40 does not match chain");
        let drained = store.drain_routine_alerts();
        assert_eq!(drained.len(), 1);
        assert!(drained[0].1.contains("row 40"));
        assert!(store.drain_routine_alerts().is_empty(), "alerts deliver once");
        // Review routines share the schedule but carry an agent + task instead
        // of a command, and render as such in the listing.
        store.upsert_review_routine("site-outsider-review", "critic", "Critique khanbot.fun like a first-time visitor.", 86400, "layout drift");
        let due = store.due_routines(2000);
        let review = due.iter().find(|r| r.0 == "site-outsider-review").expect("review routine due");
        assert_eq!(review.1, "");
        assert_eq!(review.2, "critic");
        assert!(review.3.contains("first-time visitor"));
        let listed = store.list_routines();
        let row = listed.iter().find(|r| r.0 == "site-outsider-review").unwrap();
        assert!(row.1.starts_with("review by critic:"), "listing shows the review shape: {}", row.1);
        // Same-name replace and delete work across both kinds.
        store.upsert_review_routine("site-outsider-review", "critic2", "New brief.", 7200, "");
        assert!(store.delete_routine("site-outsider-review"));
        // Removal unschedules.
        assert!(store.delete_routine("claim-verify"));
        assert!(store.due_routines(9999).is_empty());
    }

    #[test]
    fn model_stats_report_latency_and_failures() {
        let store = crate::state::Store::open(":memory:").unwrap();
        store.record_model_call("bu0y/fast", 2_000, true, "", crate::llm::Usage::default());
        store.record_model_call("bu0y/fast", 4_000, true, "", crate::llm::Usage::default());
        store.record_model_call("bu0y/slow", 90_000, true, "", crate::llm::Usage::default());
        store.record_model_call("bu0y/slow", 70_000, false, "429 rate limited", crate::llm::Usage::default());
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
        let _env = ENV_LOCK.lock().unwrap();
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

    #[test]
    fn sql_error_for_bad_name_carries_the_real_schema() {
        // Agents guessed column names (mint/symbol against positions' actual
        // asset/note) and burned a model iteration per guess — the error now
        // teaches the schema in the same reply.
        let cfg: crate::config::Config = toml::from_str(
            "ceo_model = \"p/m\"\n[[providers]]\nname = \"p\"\nbase_url = \"http://x\"\napi_key_env = \"X\"\npaid_models = [\"m\"]\n",
        )
        .unwrap();
        let root = std::env::temp_dir().join("khan-sql-hint-test");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let ctx = crate::tools::ToolCtx {
            cfg,
            store: std::sync::Arc::new(crate::state::Store::open(":memory:").unwrap()),
            workspace: root.clone(),
            http: reqwest::Client::new(),
            http_proxy: None,
        };
        crate::tools::sql::run(&ctx, "CREATE TABLE positions(id INTEGER, asset TEXT, note TEXT)").unwrap();
        let err = crate::tools::sql::run(&ctx, "SELECT mint FROM positions").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("no such column"), "original error kept: {msg}");
        assert!(msg.contains("positions(id, asset, note)"), "schema hint attached: {msg}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A truncated answer must be recognisable as such, because the three callers
    /// each treat it differently from an ordinary failure: no model fallback, no
    /// dead employee, no silent CEO retry.
    #[test]
    fn truncation_is_distinguishable_from_an_ordinary_failure() {
        use crate::llm::{truncation, Truncated};
        let t = anyhow::Error::new(Truncated { max_tokens: 16_384, reasoning_tokens: 16_000, gateway_capped: false })
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
        store.record_model_call("pay/big", 1200, true, "", crate::llm::Usage::default());
        let seen = store.models_seen();
        assert_eq!(seen, vec!["pay/big".to_string()]);
        let untried = cfg.untried_models(&seen);
        assert!(!untried.contains(&"pay/big".to_string()), "a measured model is not untried");
        assert!(untried.contains(&"pay/fresh".to_string()), "{untried:?}");
        // Free models count too — an untried free model is also unmeasured.
        assert!(untried.contains(&"or/cheap:free".to_string()), "{untried:?}");
        // A failed call is still a measurement: it produced data either way.
        store.record_model_call("pay/fresh", 500, false, "boom", crate::llm::Usage::default());
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
