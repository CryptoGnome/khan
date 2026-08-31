use crate::agent::Orchestrator;
use crate::state::Store;
use crate::tools::shell;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// How much of a failing routine's output reaches the CEO. Enough to diagnose,
/// bounded so a chatty script cannot flood the context.
const ALERT_TAIL: usize = 1500;

/// Scheduled checks the binary runs itself: management by exception.
///
/// Two kinds share one schedule:
/// - Shell routines (add_routine): a command runs here with the scrubbed env
///   and timeout the shell tool uses, at zero model cost. A pass is silent; a
///   nonzero exit, timeout, or ALERT in the output lands in the CEO's inbox.
/// - Review routines (add_review_routine): an AGENT is dispatched with a
///   stored task — judgment on a schedule (page critiques, code audits). The
///   report flows back through normal report routing, so findings reach the
///   objective's owner or the CEO like any other dispatch.
pub async fn serve(store: Arc<Store>, workspace: PathBuf, orch: Arc<Orchestrator>) {
    loop {
        tokio::time::sleep(Duration::from_secs(30)).await;
        let now = chrono::Utc::now().timestamp();
        for (name, command, agent, task, owner) in store.due_routines(now) {
            if !agent.is_empty() {
                let status = match orch.dispatch_review(&name, &agent, &task).await {
                    Ok(true) => "dispatched".to_string(),
                    // Busy/missing agents wait a full interval rather than
                    // retrying every tick: reviews are periodic, not urgent.
                    Ok(false) => "skipped (agent busy)".to_string(),
                    Err(e) => {
                        store.add_routine_alert(&name, &format!("review routine could not dispatch: {e}"));
                        format!("ALERT: {e}")
                    }
                };
                store.mark_routine_run(&name, chrono::Utc::now().timestamp(), &status);
                continue;
            }
            let o = shell::run_in_dir(&workspace, &command, Default::default())
                .await
                .unwrap_or_else(|e| shell::ShellOutcome {
                    text: format!("ERROR: routine failed to start: {e:#}"),
                    success: false,
                    timed_out: false,
                });
            let out = o.text;
            let alert = !o.success || out.contains("ALERT");
            // A hang is its own status, not just a failure: a routine that times
            // out every cycle needs a different fix than one that exits nonzero.
            let status = if o.timed_out { "ALERT (timeout)" } else if alert { "ALERT" } else { "ok" };
            store.mark_routine_run(&name, chrono::Utc::now().timestamp(), status);
            if alert {
                let tail: String = out
                    .chars()
                    .skip(out.chars().count().saturating_sub(ALERT_TAIL))
                    .collect();
                store.log("routine", "alert", &format!("{name}: {}", tail.chars().take(200).collect::<String>()));
                // An owned routine's alert is the owner's work, not the CEO's:
                // dispatch them with the alert text and let their report flow
                // through normal routing. The CEO was the sole inbox for every
                // alert and self-triaged accordingly (books drift by hand,
                // 2026-08-31). Falls back to the CEO inbox when the owner is
                // busy, missing, or the dispatch fails — an alert must never
                // be silently dropped.
                let dispatched = if owner.is_empty() {
                    false
                } else {
                    let task = format!(
                        "Routine '{name}' ALERTED — you own this domain. Alert output:\n{tail}\n\nDiagnose and handle it within your lane (fix, book the correction, or escalate with specifics), and report what you found and did."
                    );
                    matches!(orch.dispatch_review(&name, &owner, &task).await, Ok(true))
                };
                if !dispatched {
                    store.add_routine_alert(&name, &tail);
                }
            }
        }
    }
}
