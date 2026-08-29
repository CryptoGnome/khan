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
/// Agents register a shell command with an interval (add_routine); it runs here
/// on schedule with the same scrubbed env and timeout the shell tool uses, and
/// costs zero model tokens. A pass is silent. A run that exits nonzero, times
/// out, or prints ALERT lands in the CEO's inbox as a routine alert — model
/// attention is spent only when something deviates.
pub async fn serve(store: Arc<Store>, workspace: PathBuf) {
    loop {
        tokio::time::sleep(Duration::from_secs(30)).await;
        let now = chrono::Utc::now().timestamp();
        for (name, command) in store.due_routines(now) {
            let (out, ok) = shell::run_in_dir(&workspace, &command, Default::default())
                .await
                .unwrap_or_else(|e| (format!("ERROR: routine failed to start: {e:#}"), false));
            let alert = !ok || out.contains("ALERT");
            store.mark_routine_run(&name, chrono::Utc::now().timestamp(), if alert { "ALERT" } else { "ok" });
            if alert {
                let tail: String = out
                    .chars()
                    .skip(out.chars().count().saturating_sub(ALERT_TAIL))
                    .collect();
                store.add_routine_alert(&name, &tail);
                store.log("routine", "alert", &format!("{name}: {}", tail.chars().take(200).collect::<String>()));
            }
        }
    }
}
