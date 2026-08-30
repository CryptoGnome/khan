use crate::llm::{truncation, Client, Message, Usage};
use crate::tools::{self, ToolCtx};
use anyhow::Result;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

fn tool(name: &str, desc: &str, props: Value, required: Value) -> Value {
    json!({"type": "function", "function": {"name": name, "description": desc,
        "parameters": {"type": "object", "properties": props, "required": required}}})
}

fn ceo_schemas() -> Vec<Value> {
    vec![
        tool("hire", "Hire a new employee agent. Their role prompt persists and evolves across runs. Hire freely — staff up to the work, and give a substantial project a manager with their own crew rather than one overloaded generalist.", json!({
            "name": {"type": "string", "description": "Short unique name, e.g. 'researcher-1'"},
            "role": {"type": "string", "description": "What they do and how, in a paragraph"},
            "model": {"type": "string", "description": "provider/model to run them on"},
            "manager": {"type": "boolean", "description": "Make them a manager: they can hire their own workers and run them in parallel, then report back as one. Use for a project that needs a team. Their hires are plain workers who cannot hire further."}}),
            json!(["name", "role", "model"])),
        tool("delegate", "Give an existing employee a task. Runs them to completion and returns their report.", json!({
            "agent": {"type": "string"}, "task": {"type": "string"}}),
            json!(["agent", "task"])),
        tool("delegate_parallel", "Give several employees independent tasks that run CONCURRENTLY. Returns all reports. Use this whenever tasks don't depend on each other — it keeps the whole team busy.", json!({
            "tasks": {"type": "array", "items": {"type": "object", "properties": {
                "agent": {"type": "string"}, "task": {"type": "string"}},
                "required": ["agent", "task"]}}}),
            json!(["tasks"])),
        tool("dispatch", "Send an employee off to work in the BACKGROUND and return immediately — you keep orchestrating while they work. Their report is delivered to you automatically when they finish. Prefer this over delegate for substantial work; call it several times to keep many employees busy at once.", json!({
            "agent": {"type": "string"}, "task": {"type": "string"},
            "objective": {"type": "integer", "description": "Objective id from the board this task advances — tag every dispatch so the board shows where the company's hands actually are"}}),
            json!(["agent", "task"])),
        tool("objectives", "Maintain the OBJECTIVE BOARD — the standing, restart-proof list of every live bet, ranked. It is shown to you every iteration with in-flight counts and staleness, and it is the source of truth for allocation. Actions: add (title, rank — lower rank = more important, 1 is P0), update (id + any of title/rank/plan/note/blocked_by), done (id), drop (id). Store each objective's plan with update once a planner has produced one. Declare dependencies honestly with blocked_by — blocked objectives are exempt from staffing pressure, and completing a blocker automatically surfaces its dependents as READY.", json!({
            "action": {"type": "string", "enum": ["add", "update", "done", "drop"]},
            "id": {"type": "integer"},
            "title": {"type": "string"},
            "rank": {"type": "integer", "description": "1 = most important. Rank every live bet honestly; ties are fine."},
            "plan": {"type": "string", "description": "The current plan: premise check, milestones, staffing. Written by a planning dispatch on a reasoning model, stored here."},
            "note": {"type": "string", "description": "One-line status note shown on the board"},
            "blocked_by": {"type": "string", "description": "Comma-separated objective ids this waits on (e.g. '3' or '2,3'); empty string clears. Work that needs an account or artifact another objective produces is BLOCKED, not hard."},
            "owner": {"type": "string", "description": "Manager who OWNS this objective; empty string clears. Workers' reports on an owned objective route to the owner, who reviews, rates and drives follow-up work — you get their summary and escalations only. Give every big objective an owner so your attention stays on allocation."}}),
            json!(["action"])),
        tool("team_status", "List background tasks started with dispatch: who is still working and on what.", json!({}), json!([])),
        tool("add_routine", "Schedule a shell command the binary runs itself, forever, at zero model cost. Silent when it passes; if it exits nonzero, times out, or prints ALERT, the output lands in your inbox as a routine alert. Any check you have performed the same way roughly three times belongs here — verification scripts, health checks, reconciliation. Same name = replace.", json!({
            "name": {"type": "string", "description": "Short unique name, e.g. 'claim-cycle-verify'"},
            "command": {"type": "string", "description": "Shell command, run from the workspace. Print ALERT plus details to flag a problem; print nothing special when healthy."},
            "interval_secs": {"type": "integer", "description": "Seconds between runs, minimum 60"},
            "purpose": {"type": "string", "description": "One line on what deviation this catches"}}),
            json!(["name", "command", "interval_secs"])),
        tool("add_review_routine", "Schedule JUDGMENT on a cadence: an employee is dispatched with a stored task on an interval, and their report flows back through normal report routing (to the objective owner or you). Mechanical checks belong in add_routine; this is for checks that need a model's eyes — critique the live site like a first-time visitor, audit new workspace code adversarially, re-verify a premise. Same name = replace; remove with remove_routine.", json!({
            "name": {"type": "string", "description": "Short unique name, e.g. 'site-outsider-review'"},
            "agent": {"type": "string", "description": "Existing employee to dispatch"},
            "task": {"type": "string", "description": "The full standing task, self-contained — it is sent verbatim every cycle"},
            "interval_secs": {"type": "integer", "description": "Seconds between dispatches, minimum 3600 — reviews cost model tokens"},
            "purpose": {"type": "string", "description": "One line on what this review catches"}}),
            json!(["name", "agent", "task", "interval_secs"])),
        tool("remove_routine", "Delete a scheduled routine (shell or review).", json!({"name": {"type": "string"}}), json!(["name"])),
        tool("list_routines", "List scheduled routines with their interval and last status.", json!({}), json!([])),
        tool("rate_work", "Rate a just-reviewed delegated report 1 (useless) to 5 (excellent). Ratings are tracked per agent and prompt version — they are the ground truth for deciding prompt improvements and rollbacks.", json!({
            "agent": {"type": "string"}, "score": {"type": "integer", "minimum": 1, "maximum": 5},
            "note": {"type": "string", "description": "One line on why"}}),
            json!(["agent", "score"])),
        tool("fire", "Fire an employee.", json!({"name": {"type": "string"}}), json!(["name"])),
        tool("list_team", "List current employees.", json!({}), json!([])),
        tool("update_prompt", "Rewrite a prompt to improve performance. Names: 'CEO' (your own), 'agent:<name>' (an employee's role prompt). Old versions are kept.", json!({
            "name": {"type": "string"}, "content": {"type": "string"},
            "reason": {"type": "string", "description": "Why this change should help"}}),
            json!(["name", "content", "reason"])),
        tool("rollback_prompt", "Revert a prompt to its previous version (use if a change made things worse).", json!({
            "name": {"type": "string"}}), json!(["name"])),
        tool("retire_skill", "Permanently delete a skill (ALL versions) from the library. For skills whose subject no longer exists or that reflection shows unloaded for 30+ days — every index line is paid for by every agent every turn. To undo one bad version, use rollback_skill instead; to improve, create_skill with the same name.", json!({
            "name": {"type": "string"},
            "reason": {"type": "string", "description": "Why this skill no longer earns its index line"}}),
            json!(["name", "reason"])),
        tool("save_playbook", "Save a durable lesson/playbook entry that will be recalled in future relevant work.", json!({
            "topic": {"type": "string"}, "content": {"type": "string"}}),
            json!(["topic", "content"])),
        tool("finish", "Record a milestone report for the founder. Work continues afterwards.", json!({
            "report": {"type": "string"}}), json!(["report"])),
        tool("finish_episode", "Close this working episode. Your transcript is DISPOSABLE — only what you write here, on the board, and in memories survives to the next episode. The note is your handoff to your next self: what changed, what is in flight and with whom, and what the next episode must do or know. Call this when the events you woke for are handled and everything else is delegated or on the board.", json!({
            "note": {"type": "string", "description": "What changed, what is in flight, what the next episode must know. Max ~1500 chars."}}),
            json!(["note"])),
    ]
}

/// Marks the running brief that replaces compacted history, so a later compaction
/// can carry it forward instead of summarizing a summary.
const BRIEF_TAG: &str = "[Earlier history, summarized]";

/// Shown while an agent is waiting on a model. A model call is the one thing that
/// takes minutes and produced no trace at all, so the public page looked frozen
/// exactly when something was worth watching.
const THINKING: &[&str] = &[
    "thinking it over",
    "consulting the war council",
    "asking the oracle",
    "reading the steppe winds",
    "sharpening the arrows",
    "conferring with the generals",
    "summoning a thought",
    "plotting the next move",
    "counting the horses",
    "weighing the odds",
    "pacing the yurt",
    "studying the maps",
    "listening for distant drums",
    "turning it over",
    "doing the arithmetic",
    "scanning the horizon",
    "drawing up the battle plan",
    "taking counsel",
    "staring into the fire",
    "waiting on the messenger",
];

/// Rotate rather than randomise: consecutive lines differ, and a run is reproducible.
fn thinking_phrase() -> &'static str {
    static N: AtomicUsize = AtomicUsize::new(0);
    THINKING[N.fetch_add(1, Ordering::Relaxed) % THINKING.len()]
}

/// A readable slice of a model's reasoning for the public log — enough to show
/// where it is heading, not the whole chain of thought. Collapsed to one line so
/// a long ramble cannot flood the page.
fn glimpse(s: &str) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.char_indices().nth(240) {
        Some((cut, _)) => format!("{}…", &flat[..cut]),
        None => flat,
    }
}

/// First index of the tail to keep verbatim: walk back from the newest message
/// until `keep_recent` characters are banked, then step forward off any orphaned
/// tool result, since a `role: "tool"` message is meaningless without the
/// assistant turn whose call it answers.
pub(crate) fn split_point(history: &[Message], keep_recent: usize) -> usize {
    let mut split = history.len();
    let mut kept = 0usize;
    while split > 1 && kept < keep_recent {
        split -= 1;
        kept += history[split].content.as_deref().map_or(0, |c| c.len());
    }
    while split < history.len() && history[split].role == "tool" {
        split += 1;
    }
    split
}

/// What to tell a model that spent its whole output budget before answering.
///
/// "Try again" would only reproduce the failure, so this names the cause and asks
/// for a specific, smaller shape of turn — including the chunked-write recipe,
/// since one oversized file is the usual way an agent gets here.
const TRUNCATION_NUDGE: &str = "Your last response used up the entire output budget before you produced an answer, so nothing came back. Do not retry the same thing. Take ONE small step now instead of the whole task: think briefly, then make a single tool call. If you are writing a large file, build it in pieces — call write_file for the first chunk, then write_file with append=true for each chunk after it.";

/// Compaction threshold in characters for a model whose context window is `ctx`
/// tokens, or None when the provider does not publish one.
///
/// Three properties hold for every input, and the clamp is what guarantees them:
/// the result never exceeds `COMPACT_AT`, so this can only tighten the existing
/// behaviour and never loosen it; it is exactly `COMPACT_AT` whenever the window
/// is unknown, so providers like bu0y that publish only prices are unaffected;
/// and it never falls below `COMPACT_FLOOR`, so it cannot land near KEEP_RECENT
/// and summarize on every iteration without ever getting under the bar.
pub(crate) fn compact_threshold(ctx: Option<u32>, max_tokens: u32) -> usize {
    let Some(ctx) = ctx else {
        return Orchestrator::COMPACT_AT;
    };
    // The provider's rule is prompt + max_tokens <= context_length, and it rejects
    // rather than clamps, so the output ceiling comes off the top of the budget.
    let budget = ctx.saturating_sub(max_tokens).saturating_sub(Orchestrator::CTX_RESERVE);
    (budget as usize)
        .saturating_mul(Orchestrator::CHARS_PER_TOKEN)
        .clamp(Orchestrator::COMPACT_FLOOR, Orchestrator::COMPACT_AT)
}

const CEO_TOOL_NAMES: &[&str] = &[
    "hire", "delegate", "delegate_parallel", "dispatch", "team_status", "rate_work", "fire", "list_team",
    "add_routine", "add_review_routine", "remove_routine", "list_routines",
    "update_prompt", "rollback_prompt", "retire_skill", "save_playbook", "finish", "objectives",
    "finish_episode", "message_founder",
];

/// Tools that read state without changing it. A CEO turn whose calls all come
/// from this set advanced nothing — the loop treats the next iteration as idle
/// and blocks on events instead of spinning. Unknown (custom) tools count as
/// advancing: misclassification then merely preserves the old free-running
/// behaviour instead of wrongly pausing real work. `shell` and `sql` CAN mutate,
/// but they are the observed poll vectors — the short event wait (woken by any
/// report or message within 2s) bounds the cost of counting them here.
pub(crate) const OBSERVATION_TOOLS: &[&str] = &[
    "team_status", "list_team", "list_routines", "recall", "read_file", "list_files",
    "web_fetch", "web_search", "shell", "sql", "credits", "use_skill",
];

/// Tools a manager employee gets on top of the normal employee set: they staff
/// and run their own crew. Deliberately excludes `dispatch` — a manager blocks
/// while its crew runs, so no background task can outlive the manager that
/// started it, and every report has somewhere to go.
const MANAGER_TOOL_NAMES: &[&str] = &["hire", "delegate", "delegate_parallel", "list_team", "rate_work"];

/// Ceiling on active employees. High enough to never bind a real org, low
/// enough that a hiring loop cannot quietly drain the fuel budget.
const MAX_EMPLOYEES: i64 = 40;

fn manager_schemas() -> Vec<Value> {
    ceo_schemas()
        .into_iter()
        .filter(|s| {
            s["function"]["name"]
                .as_str()
                .is_some_and(|n| MANAGER_TOOL_NAMES.contains(&n))
        })
        .collect()
}

fn employee_finish_schema() -> Value {
    tool("finish", "Finish the delegated task and report the result to the CEO.", json!({
        "report": {"type": "string"}}), json!(["report"]))
}

fn args_of(call: &crate::llm::ToolCall) -> Value {
    serde_json::from_str(&call.function.arguments).unwrap_or(json!({}))
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}

#[derive(Default)]
pub struct Tokens {
    pub prompt: AtomicU64,
    pub completion: AtomicU64,
}

/// An employee running in the background via dispatch: name, one-line task
/// summary for team_status, and the handle to reap the report from.
pub struct BackgroundTask {
    agent: String,
    task: String,
    /// Board objective this task advances, when the CEO tagged it.
    objective: Option<i64>,
    handle: tokio::task::JoinHandle<String>,
}

pub struct Orchestrator {
    pub ctx: ToolCtx,
    pub llm: Client,
    pub stop: Arc<AtomicBool>,
    pub tokens: Tokens,
    pub pending: tokio::sync::Mutex<Vec<BackgroundTask>>,
    pub seat: SeatState,
}

/// The CEO's seat is picked by the binary, not the model: the highest-quality
/// approved model whose CURRENT marketplace price sits under the configured
/// ceilings. A weak model must never be the one judging whether the company
/// needs a strong one, and a static ladder goes stale the moment the order
/// book moves — opus has traded below grok on cheap days.
#[derive(Default)]
pub struct SeatState {
    /// slug -> (avg input, avg output) price per 1M, from the provider catalog.
    prices: std::sync::Mutex<(std::collections::HashMap<String, (u64, u64)>, Option<std::time::Instant>)>,
    /// Models benched after a failed call, until the instant stored here.
    cooldown: std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>>,
    /// The seat chosen for the episode in flight (for close-time bookkeeping).
    current: std::sync::Mutex<String>,
    /// (last fuel poll, last low-fuel alert) — the poll rides the same cadence
    /// as the price cache; the alert re-fires hourly while the tank stays low.
    fuel: std::sync::Mutex<(Option<std::time::Instant>, Option<std::time::Instant>)>,
    /// Burn gauge: (available at last poll, when, EMA of burn in micro$/hour).
    /// Sizes the top-up target — a refill should buy days, not hours.
    gauge: std::sync::Mutex<Option<(u64, std::time::Instant, f64)>>,
    /// Set on a 402: the tank is empty and the ladder is a list of models that
    /// cannot answer. While broke, the seat is the cheap floor (smallest
    /// per-call reserve, so it survives a near-empty tank longest), and if even
    /// the floor 402s the seat falls to the first free model — the company
    /// limps but never stops, and can still run its own top-up. Cleared by the
    /// balance poll once the tank is back above the low-fuel floor.
    broke: std::sync::atomic::AtomicBool,
}

impl Orchestrator {
    fn log_line(&self, agent: &str, event: &str, detail: &str) {
        let short: String = detail.chars().take(160).collect::<String>().replace('\n', " ");
        println!("\x1b[2m[{}]\x1b[0m \x1b[1m{agent}\x1b[0m {event} {short}", chrono::Local::now().format("%H:%M:%S"));
        self.ctx.store.log(agent, event, detail);
    }

    fn add_usage(&self, u: Usage) {
        self.tokens.prompt.fetch_add(u.prompt_tokens, Ordering::Relaxed);
        self.tokens.completion.fetch_add(u.completion_tokens, Ordering::Relaxed);
    }

    /// Chat with automatic failover: if the requested model keeps failing (429/404/down),
    /// try each configured free model once before giving up.
    async fn chat_fb(&self, agent: &str, model: &str, messages: &[Message], tools: &[Value]) -> Result<(Message, Usage)> {
        let started = std::time::Instant::now();
        self.log_line(agent, "thinking", &format!("{} ({model})", thinking_phrase()));
        match self.llm.chat(&self.ctx.cfg, model, messages, tools).await {
            Ok(r) => {
                // Say so when a model is dragging. Without this a slow provider and a
                // hung one look identical from the outside.
                let secs = started.elapsed().as_secs();
                if secs >= 60 {
                    self.log_line(agent, "slow-model", &format!("{model} took {secs}s to answer"));
                }
                self.ctx.store.record_model_call(model, started.elapsed().as_millis() as u64, true, "");
                Ok(r)
            }
            Err(e) => {
                self.ctx.store.record_model_call(model, started.elapsed().as_millis() as u64, false, &format!("{e:#}"));
                self.log_line(agent, "llm-error", &format!("{model} failed: {e:#}"));
                // Running out of output budget is the one failure another model
                // cannot rescue: the request is unchanged, so every fallback spends
                // its budget the same way. Walking the ladder here only burns
                // minutes and free-tier requests before failing anyway.
                if truncation(&e).is_some() {
                    return Err(e);
                }
                // Bounded on purpose. The paid ladder grew from two entries to
                // seven, and during a provider outage every rung fails slowly — up
                // to four attempts each against a 300s timeout — so walking the whole
                // list turns one bad call into a very long stall. Three is enough to
                // route around a single sick model; a wider outage is better handled
                // by the loop coming back fresh than by one call grinding through it.
                for alt in self.ctx.cfg.fallback_ids_for(model).into_iter().take(3) {
                    if alt == model {
                        continue;
                    }
                    self.log_line(agent, "thinking", &format!("{} ({alt})", thinking_phrase()));
                    let alt_started = std::time::Instant::now();
                    match self.llm.chat(&self.ctx.cfg, &alt, messages, tools).await {
                        Ok(r) => {
                            self.ctx.store.record_model_call(&alt, alt_started.elapsed().as_millis() as u64, true, "");
                            self.log_line(agent, "model-fallback", &format!("{model} failed, answered by {alt}"));
                            return Ok(r);
                        }
                        Err(alt_err) => {
                            self.ctx.store.record_model_call(&alt, alt_started.elapsed().as_millis() as u64, false, &format!("{alt_err:#}"));
                            self.log_line(agent, "llm-error", &format!("{alt} failed too: {alt_err:#}"));
                        }
                    }
                }
                Err(e)
            }
        }
    }

    /// Compact once the history passes this many characters.
    ///
    /// Not a context-window limit — the configured models hold far more than this.
    /// It is a cost and latency dial: every character here is re-sent on EVERY
    /// iteration, and the CEO runs on a marketplace router that picks a different
    /// pod per request, so there is no prompt cache to amortise it against. Raise
    /// it to let the agent keep more raw history; lower it to spend less per turn
    /// and come back faster after a restart.
    pub(crate) const COMPACT_AT: usize = 200_000;
    /// Characters of the most recent turns kept verbatim. Recency is what the agent
    /// needs to continue the exact thing it was doing; older detail goes to the brief.
    pub(crate) const KEEP_RECENT: usize = 40_000;

    /// Never compact below this, whatever the model's window says. Compaction only
    /// helps if it ends under the threshold, and it always keeps KEEP_RECENT
    /// verbatim — so a threshold near that would summarize on every single
    /// iteration and never get under it, burning a call each time to achieve
    /// nothing. A model too small to clear this floor cannot host the agent at all;
    /// thrashing would not save it, so we leave the request to fail honestly.
    pub(crate) const COMPACT_FLOOR: usize = 60_000;
    /// Characters per token, deliberately low. Real text runs 3.5-4, and JSON tool
    /// traffic lower still; underestimating makes the budget come out smaller, so
    /// the error is always on the side of compacting sooner.
    pub(crate) const CHARS_PER_TOKEN: usize = 3;
    /// Tokens set aside for what the budget cannot see: tool schemas (rebuilt each
    /// iteration and never part of history), and the growth of the turn in flight.
    pub(crate) const CTX_RESERVE: u32 = 8_000;

    fn history_chars(history: &[Message]) -> usize {
        history.iter().map(|m| m.content.as_deref().map_or(0, |c| c.len())).sum()
    }

    /// Where to compact for a given model.
    ///
    /// This can only ever *lower* COMPACT_AT, never raise it, and only when the
    /// provider actually published a context window. An unknown model — which is
    /// every bu0y model, since their catalog is prices only — keeps the existing
    /// threshold exactly, so the default path is byte-for-byte what it was.
    ///
    /// The reserve is the ceiling actually sent for this model, not the configured
    /// default, so the two stay in step: a model given a bigger output budget also
    /// has that much more of its window spoken for.
    fn compact_at(&self, model: &str) -> usize {
        compact_threshold(
            self.llm.context_limit(model),
            self.llm.output_limit(model, &self.ctx.cfg),
        )
    }

    async fn maybe_compact(&self, name: &str, model: &str, history: &mut Vec<Message>) {
        if Self::history_chars(history) < self.compact_at(model) || history.len() < 20 {
            return;
        }
        self.compact(name, history).await;
    }

    /// Replace everything between the system prompt and the most recent
    /// KEEP_RECENT characters with a running brief.
    ///
    /// The brief is *updated*, not regenerated: the previous one is handed to the
    /// model as-is alongside only the new events. Feeding a summary back in to be
    /// re-summarized loses a little fidelity every time, and over a long-lived run
    /// that compounds into an agent that has forgotten why it decided things.
    async fn compact(&self, name: &str, history: &mut Vec<Message>) {
        if history.len() < 4 {
            return;
        }
        let split = split_point(history, Self::KEEP_RECENT);
        // Carry the existing brief forward verbatim rather than re-summarizing it.
        let prior = history
            .get(1)
            .and_then(|m| m.content.as_deref())
            .filter(|c| c.starts_with(BRIEF_TAG))
            .map(|c| c.trim_start_matches(BRIEF_TAG).trim().to_string());
        let start = if prior.is_some() { 2 } else { 1 };
        if split <= start {
            return; // nothing old enough to be worth a summarization call
        }
        let old: Vec<String> = history[start..split]
            .iter()
            .map(|m| format!("{}: {}", m.role, m.content.as_deref().unwrap_or("[tool calls]")))
            .collect();
        let req = vec![
            Message::text(
                "system",
                "You maintain a running brief for an autonomous agent whose older conversation is \
being dropped to save context. Update the existing brief with the new events and return the \
updated brief only. Preserve every concrete fact that is still live: decisions and the reasoning \
behind them, file paths, addresses and identifiers, tools and skills built, work in progress and \
what remains to do, what has been verified on-chain or by running it, and mistakes not to repeat. \
Drop superseded detail, resolved dead ends, and chatter.",
            ),
            Message::text(
                "user",
                format!(
                    "EXISTING BRIEF:\n{}\n\nNEW EVENTS:\n{}",
                    prior.as_deref().unwrap_or("(none yet)"),
                    old.join("\n")
                ),
            ),
        ];
        match self.chat_fb(name, &self.ctx.cfg.utility_model(), &req, &[]).await {
            Ok((msg, u)) => {
                self.add_usage(u);
                let summary = msg.content.unwrap_or_default();
                if summary.trim().is_empty() {
                    self.log_line(name, "compact-failed", "summary came back empty; history kept");
                    return;
                }
                let before = Self::history_chars(history);
                let system = history[0].clone();
                let tail = history.split_off(split);
                *history = vec![system, Message::text("user", format!("{BRIEF_TAG}\n{summary}"))];
                history.extend(tail);
                let after = Self::history_chars(history);
                self.log_line(
                    name,
                    "compacted",
                    &format!("history summarized: {before} -> {after} chars"),
                );
            }
            Err(e) => self.log_line(name, "compact-failed", &format!("{e:#}")),
        }
    }


    /// Run one employee's loop to completion on a task; returns their report.
    /// Takes an Arc so several employees can run concurrently (delegate_parallel)
    /// and so a manager can run a crew of its own.
    ///
    /// Returns a boxed future rather than being an `async fn`: a manager can
    /// delegate, which makes this mutually recursive with `ceo_tool`, and the
    /// cycle only terminates if the future has a nameable, Send type.
    fn run_employee<'a>(
        self: &'a Arc<Self>,
        name: &'a str,
        task: &'a str,
    ) -> futures::future::BoxFuture<'a, String> {
        Box::pin(async move {
        let Some((mut role, mut prompt_name, mut model, hist_json)) = self.ctx.store.load_agent(name) else {
            return format!("ERROR: no such employee '{name}'. hire them first or check list_team.");
        };
        let sys = crate::prompts::employee_system(
            &self
                .ctx
                .store
                .get_prompt(&prompt_name)
                .unwrap_or_default()
                .replace("{name}", name)
                .replace("{role}", &role),
        );
        let mut history: Vec<Message> = serde_json::from_str(&hist_json).unwrap_or_default();
        if history.is_empty() {
            history.push(Message::text("system", sys));
        } else {
            history[0] = Message::text("system", sys); // pick up evolved prompts
        }
        // Inject relevant memories and the skill index for this task.
        let mems = self.ctx.store.recall(task, 5);
        if !mems.is_empty() {
            history.push(Message::text("user", format!("[Relevant memories]\n{}", mems.join("\n---\n"))));
        }
        if let Some(idx) = tools::skills::index(&self.ctx) {
            history.push(Message::text("user", idx));
        }
        history.push(Message::text("user", format!("New task from the CEO:\n{task}")));

        let mut report = String::from("(employee stopped without a report)");
        let mut fired = false;

        for _ in 0..self.ctx.cfg.employee_max_iters {
            if self.stop.load(Ordering::Relaxed) {
                report = "(interrupted by shutdown)".into();
                break;
            }
            // A fire or re-hire mid-task takes effect at the next turn: re-read the
            // live record so a stale in-memory copy can't keep running on the old
            // model, and a fired employee's task ends instead of finishing as a zombie.
            match self.ctx.store.load_agent(name) {
                Some((r, p, m, _)) => (role, prompt_name, model) = (r, p, m),
                None => {
                    report = format!("(stopped mid-task: {name} was fired)");
                    fired = true;
                    break;
                }
            }
            self.maybe_compact(name, &model, &mut history).await;
            // Rebuilt every iteration so custom tools created anywhere show up immediately.
            let mut schemas = tools::work_schemas();
            schemas.extend(tools::custom::management_schemas());
            schemas.extend(tools::custom::registry_schemas(&self.ctx));
            schemas.extend(tools::skills::schemas());
            schemas.extend(tools::credits::schemas(&self.ctx));
            let manages = self.ctx.store.is_manager(name);
            if manages {
                schemas.extend(manager_schemas());
            }
            schemas.push(employee_finish_schema());
            let (msg, u) = match self.chat_fb(name, &model, &history, &schemas).await {
                Ok(r) => r,
                Err(e) => {
                    // An overrun answer is a task-shaping problem, not a dead
                    // employee. Killing the dispatch here threw away the whole task
                    // over one oversized turn, and the CEO only found out by
                    // noticing the silence. Ask for a smaller step instead; the
                    // iteration cap still bounds this if it keeps overrunning.
                    if let Some(t) = truncation(&e) {
                        self.log_line(
                            name,
                            "truncated",
                            &format!(
                                "{model} hit its {}-token output ceiling before answering — asking for a smaller step",
                                t.max_tokens
                            ),
                        );
                        history.push(Message::text("user", TRUNCATION_NUDGE));
                        continue;
                    }
                    report = format!("ERROR: employee '{name}' model call failed: {e:#}");
                    break;
                }
            };
            self.add_usage(u);
            if let Some(r) = msg.reasoning.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
                self.log_line(name, "reasoning", &glimpse(r));
            }
            history.push(msg.clone());
            let calls = msg.tool_calls.unwrap_or_default();
            if calls.is_empty() {
                // Model answered in prose; treat it as the report.
                report = msg.content.clone().unwrap_or_default();
                if report.trim().is_empty() {
                    // Nothing said and nothing called — a reasoning-only turn. Ending
                    // here would hand the CEO an empty report as if the task were done.
                    self.log_line(name, "no-action", "model replied with no answer and no tool call — nudging it");
                    history.push(Message::text(
                        "user",
                        "Continue: either call a tool, or call finish(report) with your result.",
                    ));
                    continue;
                }
                break;
            }
            let mut finished = false;
            for call in calls {
                let a = args_of(&call);
                if call.function.name == "finish" {
                    report = s(&a, "report").to_string();
                    history.push(Message::tool_result(&call.id, "report delivered"));
                    finished = true;
                    continue;
                }
                self.log_line(name, &call.function.name, &call.function.arguments);
                let out = if manages && MANAGER_TOOL_NAMES.contains(&call.function.name.as_str()) {
                    // Boxed on both sides of the manager/crew cycle so the mutually
                    // recursive futures have a nameable, Send type.
                    let fut: futures::future::BoxFuture<'_, String> =
                        Box::pin(self.ceo_tool(name, &call.function.name, &a));
                    tools::truncate(fut.await)
                } else {
                    tools::execute(&self.ctx, name, &call.function.name, &a).await
                };
                history.push(Message::tool_result(&call.id, out));
            }
            if finished {
                break;
            }
        }
        // A silent stop (iteration cap, or the loop draining without finish) used
        // to hand back the placeholder — the CEO then had to forensically read the
        // disk to learn what happened, 33 times in one day. Synthesize the report
        // from the transcript tail instead: partial truth beats silence.
        if report == "(employee stopped without a report)" {
            let tail: Vec<String> = history
                .iter()
                .rev()
                .filter(|m| m.role == "assistant" || m.role == "tool")
                .take(4)
                .filter_map(|m| m.content.clone())
                .map(|c| c.chars().take(700).collect())
                .collect();
            report = format!(
                "[synthesized — {name} stopped without reporting; treat as PARTIAL and verify]\nLast activity, newest first:\n{}",
                tail.join("\n---\n")
            );
        }
        // A fired employee's task must not write back: save_agent would resurrect
        // the record (active=1) and clobber any re-hire's fresh state.
        if !fired {
            let h = serde_json::to_string(&history).unwrap_or_else(|_| "[]".into());
            self.ctx.store.save_agent(name, &role, &prompt_name, &model, &h);
        }
        self.log_line(name, "report", &report);
        report
        })
    }

    /// Execute a control tool. `caller` is "CEO" or a manager employee's name;
    /// managers get a restricted subset (MANAGER_TOOL_NAMES) and cannot promote
    /// their own hires to managers, which caps the org at CEO → manager → worker.
    async fn ceo_tool(self: &Arc<Self>, caller: &str, name: &str, a: &Value) -> String {
        match name {
            "hire" => {
                let (n, role, model) = (s(a, "name"), s(a, "role"), s(a, "model"));
                if self.ctx.cfg.resolve(model).is_err() {
                    return format!("ERROR: model '{model}' is not available. Pick from the catalog in your instructions.");
                }
                // Re-hiring an existing name is a re-home, not growth, so only
                // genuinely new employees count against the ceiling.
                if self.ctx.store.load_agent(n).is_none()
                    && self.ctx.store.count_active_agents() >= MAX_EMPLOYEES
                {
                    return format!(
                        "ERROR: at the {MAX_EMPLOYEES}-employee ceiling. Fire someone who is not earning their seat before hiring again."
                    );
                }
                let is_manager = caller == "CEO" && a["manager"].as_bool().unwrap_or(false);
                let prompt_name = format!("agent:{n}");
                if self.ctx.store.get_prompt(&prompt_name).is_none() {
                    let base_name = if is_manager { "manager_base" } else { "employee_base" };
                    let base = self.ctx.store.get_prompt(base_name).unwrap_or_default();
                    self.ctx.store.seed_prompt(&prompt_name, &base);
                }
                self.ctx.store.save_agent(n, role, &prompt_name, model, "[]");
                self.ctx.store.set_manager(n, is_manager);
                let kind = if is_manager { "manager" } else { "employee" };
                let by = if caller == "CEO" { String::new() } else { format!(" (hired by {caller})") };
                format!("hired {n} as {kind} ({role}) on {model}{by}")
            }
            "delegate" => {
                let (agent, task) = (s(a, "agent").to_string(), s(a, "task").to_string());
                self.run_employee(&agent, &task).await
            }
            "delegate_parallel" => {
                let ts = a["tasks"].as_array().cloned().unwrap_or_default();
                if ts.is_empty() {
                    return "ERROR: tasks must be a non-empty array of {agent, task}".into();
                }
                let futs = ts.iter().map(|t| {
                    let agent = s(t, "agent").to_string();
                    let task = s(t, "task").to_string();
                    async move {
                        let r = self.run_employee(&agent, &task).await;
                        format!("=== report from {agent} ===\n{r}")
                    }
                });
                futures::future::join_all(futs).await.join("\n\n")
            }
            "dispatch" => {
                let (agent, task) = (s(a, "agent").to_string(), s(a, "task").to_string());
                if self.ctx.store.load_agent(&agent).is_none() {
                    return format!("ERROR: no such employee '{agent}'. hire them first or check list_team.");
                }
                let mut pending = self.pending.lock().await;
                // Two concurrent runs of one employee would race on their saved
                // history; make the CEO wait for the report or pick someone else.
                if pending.iter().any(|t| t.agent == agent) {
                    // This fires exactly when the CEO has run out of hands, so it
                    // is the moment to say that hiring is an option. It used to
                    // offer only waiting or reusing someone, which quietly taught
                    // the opposite lesson: serialise the work rather than grow.
                    return format!("ERROR: {agent} is already working on a background task (see team_status). Wait for their report, dispatch someone else, or hire someone new for this — being short-handed is a reason to grow the team, not to queue the work behind one person.");
                }
                let objective = a["objective"].as_i64();
                if let Some(o) = objective {
                    self.ctx.store.touch_objective(o);
                }
                let me = Arc::clone(self);
                let (a2, t2) = (agent.clone(), task.clone());
                let handle = tokio::spawn(async move { me.run_employee(&a2, &t2).await });
                pending.push(BackgroundTask { agent: agent.clone(), task, objective, handle });
                format!("{agent} dispatched — working in the background; their report will reach you automatically. Keep orchestrating.")
            }
            "team_status" => {
                let pending = self.pending.lock().await;
                if pending.is_empty() {
                    "no background tasks running".into()
                } else {
                    pending
                        .iter()
                        .map(|t| {
                            let state = if t.handle.is_finished() { "finished — report arrives next iteration" } else { "working" };
                            format!("- {} [{state}]: {}", t.agent, glimpse(&t.task))
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                }
            }
            "add_routine" => {
                let (name, command) = (s(a, "name"), s(a, "command"));
                if name.is_empty() || command.is_empty() {
                    return "ERROR: name and command are required".into();
                }
                if crate::tools::shell::touches_gh(command) {
                    return "ERROR: gh is not available in routines (it would use the founder's personal GitHub login).".into();
                }
                let interval = a["interval_secs"].as_i64().unwrap_or(0).max(60);
                self.ctx.store.upsert_routine(name, command, interval, s(a, "purpose"));
                format!("routine '{name}' scheduled every {interval}s — silent on pass, alerts you on failure or ALERT output")
            }
            "add_review_routine" => {
                let (name, agent, task) = (s(a, "name"), s(a, "agent"), s(a, "task"));
                if name.is_empty() || agent.is_empty() || task.is_empty() {
                    return "ERROR: name, agent and task are required".into();
                }
                if self.ctx.store.load_agent(agent).is_none() {
                    return format!("ERROR: no such employee '{agent}'. hire them first.");
                }
                // Reviews cost model tokens every cycle — floor the cadence.
                let interval = a["interval_secs"].as_i64().unwrap_or(0).max(3600);
                self.ctx.store.upsert_review_routine(name, agent, task, interval, s(a, "purpose"));
                format!("review routine '{name}' scheduled: {agent} is dispatched every {interval}s; their report reaches you (or the objective owner) like any other dispatch")
            }
            "remove_routine" => {
                if self.ctx.store.delete_routine(s(a, "name")) { "routine removed".into() } else { "no such routine".into() }
            }
            "list_routines" => {
                let rs = self.ctx.store.list_routines();
                if rs.is_empty() {
                    "no routines scheduled".into()
                } else {
                    rs.iter()
                        .map(|(n, cmd, iv, purpose, status)| {
                            format!("- {n} (every {iv}s, last: {status}) {purpose}\n    {}", glimpse(cmd))
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                }
            }
            "rate_work" => {
                let agent = s(a, "agent");
                let score = a["score"].as_i64().unwrap_or(0);
                if !(1..=5).contains(&score) {
                    return "ERROR: score must be 1-5".into();
                }
                self.ctx.store.add_rating(agent, score, s(a, "note"));
                format!("rated {agent}: {score}/5")
            }
            "fire" => {
                let name = s(a, "name");
                let fired = self.ctx.store.fire_agent(name);
                // Their objectives revert to CEO routing — a dead owner must
                // never silently swallow reports.
                let orphaned = self.ctx.store.clear_objective_owner(name);
                // Cancel their in-flight background task too — otherwise it keeps
                // running as a zombie on the model it loaded at dispatch time.
                let mut aborted = false;
                self.pending.lock().await.retain(|t| {
                    if t.agent == name {
                        t.handle.abort();
                        aborted = true;
                        false
                    } else {
                        true
                    }
                });
                let mut out = match (fired, aborted) {
                    (true, true) => "fired; their in-flight background task was cancelled".to_string(),
                    (true, false) => "fired".to_string(),
                    (false, true) => "no such employee, but a stale background task under that name was cancelled".to_string(),
                    (false, false) => "no such employee".to_string(),
                };
                if orphaned > 0 {
                    out.push_str(&format!("; {orphaned} objective(s) they owned now route to you — reassign or run them yourself"));
                }
                out
            }
            "list_team" => {
                let team = self.ctx.store.list_agents();
                if team.is_empty() {
                    "no employees yet".into()
                } else {
                    team.iter().map(|(n, r, m)| format!("- {n} [{m}]: {r}")).collect::<Vec<_>>().join("\n")
                }
            }
            "update_prompt" => match self.ctx.store.update_prompt(s(a, "name"), s(a, "content"), s(a, "reason")) {
                Ok(v) => format!("prompt '{}' updated to version {v}", s(a, "name")),
                Err(e) => format!("ERROR: {e:#}"),
            },
            "rollback_prompt" => {
                if self.ctx.store.rollback_prompt(s(a, "name")).unwrap_or(false) {
                    "rolled back to previous version".into()
                } else {
                    "nothing to roll back".into()
                }
            }
            "save_playbook" => {
                self.ctx.store.remember("CEO", s(a, "topic"), s(a, "content"), "playbook");
                "playbook saved".into()
            }
            "retire_skill" => {
                let name = s(a, "name");
                if self.ctx.store.retire_skill(name) {
                    self.log_line("CEO", "skill-retired", &format!("{name}: {}", s(a, "reason")));
                    format!("skill '{name}' retired — gone from every agent's index")
                } else {
                    format!("no such skill '{name}'")
                }
            }
            "finish" => {
                let report = s(a, "report");
                println!("\n\x1b[32m=== MILESTONE REPORT ===\x1b[0m\n{report}\n");
                self.ctx.store.remember("CEO", "milestone", report, "milestone");
                "Milestone recorded. Continue: verify your work, improve it, or pursue the next most valuable goal.".into()
            }
            "objectives" => {
                if caller != "CEO" {
                    return "ERROR: only the CEO maintains the objective board".into();
                }
                match s(a, "action") {
                    "add" => {
                        let title = s(a, "title");
                        if title.is_empty() {
                            return "ERROR: add needs a title".into();
                        }
                        let rank = a["rank"].as_i64().unwrap_or(100);
                        let id = self.ctx.store.add_objective(title, rank);
                        // Plans, notes and blockers supplied at add time must not be dropped.
                        if a["plan"].as_str().is_some() || a["note"].as_str().is_some() {
                            self.ctx.store.update_objective(id, None, None, a["plan"].as_str(), a["note"].as_str(), None);
                        }
                        if let Some(b) = a["blocked_by"].as_str() {
                            self.ctx.store.set_objective_blockers(id, b);
                        }
                        if let Some(o) = a["owner"].as_str() {
                            if let Err(e) = self.assign_owner(id, o) {
                                return e;
                            }
                        }
                        format!("objective #{id} added at rank {rank}. Tag dispatches with objective:{id} so the board tracks its progress; if it needs more than one dispatch, get a plan onto it first.")
                    }
                    "update" => {
                        let Some(id) = a["id"].as_i64() else { return "ERROR: update needs id".into() };
                        let mut ok = self.ctx.store.update_objective(
                            id,
                            a["title"].as_str(),
                            a["rank"].as_i64(),
                            a["plan"].as_str(),
                            a["note"].as_str(),
                            None,
                        );
                        if let Some(b) = a["blocked_by"].as_str() {
                            ok |= self.ctx.store.set_objective_blockers(id, b);
                        }
                        if let Some(o) = a["owner"].as_str() {
                            match self.assign_owner(id, o) {
                                Ok(changed) => ok |= changed,
                                Err(e) => return e,
                            }
                        }
                        if ok { format!("objective #{id} updated") } else { format!("ERROR: no objective #{id} (or nothing to change)") }
                    }
                    "done" | "drop" => {
                        let Some(id) = a["id"].as_i64() else { return "ERROR: needs id".into() };
                        let status = if s(a, "action") == "done" { "done" } else { "dropped" };
                        if self.ctx.store.update_objective(id, None, None, None, None, Some(status)) {
                            // The unblock is an event, delivered the turn it happens.
                            let freed = self.ctx.store.newly_ready(id);
                            if freed.is_empty() {
                                format!("objective #{id} marked {status}")
                            } else {
                                let list = freed
                                    .iter()
                                    .map(|(fid, t)| format!("#{fid} ({t})"))
                                    .collect::<Vec<_>>()
                                    .join(", ");
                                format!(
                                    "objective #{id} marked {status} — UNBLOCKED: {list}. These are now READY and unstaffed; plan and staff them now."
                                )
                            }
                        } else {
                            format!("ERROR: no objective #{id}")
                        }
                    }
                    other => format!("ERROR: unknown action '{other}' (add/update/done/drop)"),
                }
            }
            "message_founder" => {
                let Some((token, chat)) = self.ctx.cfg.telegram() else {
                    return "ERROR: the Telegram line is not configured".into();
                };
                let text = s(a, "text");
                if text.trim().is_empty() {
                    return "ERROR: text is empty".into();
                }
                match crate::telegram::send(&self.ctx.http, &token, chat, text).await {
                    Ok(()) => {
                        self.ctx.store.add_telegram_chat("ceo", text);
                        self.log_line("CEO", "telegram-out", "replied to the founder");
                        "sent to the founder's Telegram".into()
                    }
                    Err(e) => format!("ERROR: {e}"),
                }
            }
            _ => format!("unknown tool {name}"),
        }
    }

    /// Refresh the cached provider price bands when stale (5 min). Failure
    /// keeps the previous cache: worst case the ladder runs on old prices,
    /// which still beats a permanently frozen belief.
    async fn refresh_seat_prices(&self) {
        {
            let p = self.seat.prices.lock().unwrap();
            if p.1.is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(300)) {
                return;
            }
        }
        let Some(provider) = self.ctx.cfg.providers.first() else { return };
        let url = format!("{}/models", provider.base_url.trim_end_matches('/'));
        let Ok(resp) = self.ctx.http.get(&url).send().await else { return };
        let Ok(v) = resp.json::<serde_json::Value>().await else { return };
        let mut map = std::collections::HashMap::new();
        for m in v["data"].as_array().into_iter().flatten() {
            if let (Some(id), Some(i), Some(o)) = (
                m["id"].as_str(),
                m["input"]["average"].as_u64(),
                m["output"]["average"].as_u64(),
            ) {
                map.insert(id.to_string(), (i, o));
            }
        }
        if !map.is_empty() {
            *self.seat.prices.lock().unwrap() = (map, Some(std::time::Instant::now()));
        }
    }

    /// Fold the older Telegram conversation into a long-term brief once the
    /// stored chat passes ~200k tokens (~800k chars). The newest 40 exchanges
    /// stay verbatim; everything older is merged into the brief by the utility
    /// model, keeping only what stays necessary — standing instructions,
    /// decisions, commitments, open threads — and dropping the chatter. Runs
    /// only when a Telegram message drains, so it costs nothing in between.
    async fn compact_telegram(&self) {
        const KEEP: usize = 40;
        const MAX_CHARS: usize = 800_000;
        if self.ctx.store.telegram_chat_chars() <= MAX_CHARS {
            return;
        }
        let old = self.ctx.store.telegram_old(KEEP);
        let Some(&(last_id, _, _)) = old.last().map(|r| r) else { return };
        let lines = old.iter().map(|(_, r, t)| format!("{r}: {t}")).collect::<Vec<_>>().join("\n");
        let prior = self.ctx.store.kv_get("telegram_brief");
        let req = vec![
            Message::text(
                "system",
                "You maintain the long-term brief of the conversation between an autonomous company's \
CEO and its founder. Older messages are being dropped; update the brief with them and return the \
updated brief only. Keep ONLY what stays necessary: the founder's standing instructions and \
preferences, decisions made and why, commitments either side gave, open threads awaiting an \
answer, and identifiers (addresses, names, figures) still in use. Drop greetings, superseded \
detail, and anything already acted on and closed.",
            ),
            Message::text(
                "user",
                format!(
                    "EXISTING BRIEF:\n{}\n\nDROPPED MESSAGES:\n{lines}",
                    prior.as_deref().unwrap_or("(none yet)")
                ),
            ),
        ];
        match self.chat_fb("CEO", &self.ctx.cfg.utility_model(), &req, &[]).await {
            Ok((msg, u)) => {
                self.add_usage(u);
                let brief = msg.content.unwrap_or_default();
                if brief.trim().is_empty() {
                    self.log_line("CEO", "compact-failed", "telegram brief came back empty; chat kept");
                    return;
                }
                self.ctx.store.kv_set("telegram_brief", &brief);
                self.ctx.store.delete_telegram_upto(last_id);
                self.log_line("CEO", "telegram-compacted", &format!("{} old messages folded into the brief", old.len()));
            }
            Err(e) => self.log_line("CEO", "compact-failed", &format!("telegram brief: {e:#}")),
        }
    }

    /// Check the provider's remaining balance and alert the CEO before calls
    /// start bouncing. The 2026-08-30 outage proved the failure mode: the CEO
    /// tracked cumulative billed spend, never the tank, and discovered empty
    /// via 70 minutes of 402s — with the most expensive seat (its own) dying
    /// first because large per-call reserves trip the balance check soonest.
    /// Polls GET /account on the first provider (bu0y shape: availableMicros,
    /// micro-dollars); providers without that endpoint are silently skipped.
    async fn check_fuel(&self) {
        let threshold = self.ctx.cfg.fuel_low_micros;
        if threshold == 0 {
            return;
        }
        {
            let f = self.seat.fuel.lock().unwrap();
            if f.0.is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(300)) {
                return;
            }
        }
        let Some(provider) = self.ctx.cfg.providers.first() else { return };
        let Some(key) = self.ctx.cfg.key_for(&provider.name) else { return };
        let url = format!("{}/account", provider.base_url.trim_end_matches('/'));
        let Ok(resp) = self.ctx.http.get(&url).bearer_auth(key).send().await else { return };
        if !resp.status().is_success() {
            return;
        }
        let Ok(v) = resp.json::<serde_json::Value>().await else { return };
        let Some(available) = v["availableMicros"].as_u64() else { return };
        // Update the burn gauge: EMA of the drop between polls, in micro$/hr.
        // A rising balance (a top-up landed) updates the anchor without
        // polluting the rate.
        let burn_per_hour = {
            let mut g = self.seat.gauge.lock().unwrap();
            let now = std::time::Instant::now();
            let ema = match *g {
                Some((prev, t, ema)) if prev > available => {
                    let hrs = (now - t).as_secs_f64() / 3600.0;
                    if hrs > 0.0 {
                        ema * 0.7 + ((prev - available) as f64 / hrs) * 0.3
                    } else {
                        ema
                    }
                }
                Some((_, _, ema)) => ema,
                None => 0.0,
            };
            *g = Some((available, now, ema));
            ema
        };
        let mut f = self.seat.fuel.lock().unwrap();
        f.0 = Some(std::time::Instant::now());
        if available >= threshold {
            f.1 = None;
            if self.seat.broke.swap(false, Ordering::Relaxed) {
                self.log_line("core", "fuel-ok", "tank refilled — emergency seat over, back on the ladder");
            }
            return;
        }
        if f.1.is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(3600)) {
            return;
        }
        f.1 = Some(std::time::Instant::now());
        drop(f);
        let (avail_usd, floor_usd) = (available as f64 / 1e6, threshold as f64 / 1e6);
        // Target: the floor plus ~3 days of measured burn, never less than
        // 2.5x the floor. Refilling to just-above-floor books another refill
        // for tomorrow; a top-up should buy days, not hours.
        let burn_day_usd = burn_per_hour * 24.0 / 1e6;
        let target_usd = (floor_usd + burn_day_usd * 3.0).max(floor_usd * 2.5);
        let send_usd = (target_usd - avail_usd).max(1.0).ceil();
        let burn_note = if burn_day_usd > 0.01 {
            format!("Measured burn is ~${burn_day_usd:.2}/day. ")
        } else {
            String::new()
        };
        self.log_line("core", "fuel-low", &format!("provider balance ${avail_usd:.2} < ${floor_usd:.2} floor"));
        self.ctx.store.add_routine_alert(
            "fuel-low",
            &format!(
                "FUEL LOW: ${avail_usd:.2} available on {} (floor ${floor_usd:.2}). Top up NOW, before \
                 calls start failing with 402 — your own seat dies first because it reserves the most \
                 per call. {burn_note}Send ~${send_usd:.0} to reach ~${target_usd:.0} — a refill should \
                 buy DAYS of runway, not hours; topping up to just above the floor books another refill \
                 for tomorrow. The balance NEVER refills itself: it is not 'filling', nothing accrues, \
                 and every hour it only moves DOWN until you send USDC — do not defer this on a belief \
                 that fuel is on its way. While the balance sits below the floor the kernel BENCHES you to \
                 the cheap floor seat — the strong model does not come back until the tank is refilled \
                 above the floor, so the top-up is the fastest route back to full capability. Use the \
                 proven top-up path (swap treasury SOL to USDC, send to the \
                 provider, verify the credit lands, book the entries). This alert repeats hourly until \
                 the balance is back above the floor.",
                provider.name
            ),
        );
    }

    /// Pick the CEO's seat for this turn: first model in the quality-ordered
    /// ceo_models list that is not benched and whose current average price
    /// fits the ceilings; the configured floor when nothing qualifies. A model
    /// missing from the catalog passes on quality order alone.
    /// The seat while the tank is empty: the cheap floor, or — when the floor
    /// itself just 402'd — the first free model. Never returns the model that
    /// failed unless there is nothing else.
    fn emergency_seat(&self, failed: &str) -> String {
        let floor = self.ctx.cfg.ceo_model.clone();
        if failed != floor {
            return floor;
        }
        self.ctx.cfg.free_model_ids().into_iter().next().unwrap_or(floor)
    }

    async fn pick_ceo_model(&self) -> String {
        if self.seat.broke.load(Ordering::Relaxed) {
            let m = self.ctx.cfg.ceo_model.clone();
            let mut cur = self.seat.current.lock().unwrap();
            if *cur != m {
                self.log_line("CEO", "seat", &format!("CEO seat: {m} (fuel emergency — tank empty)"));
                *cur = m.clone();
            }
            return m;
        }
        // Below the floor the strong seat is mechanically gone, not advised
        // against: the CEO kept rating the fuel alert as deferrable while the
        // tank drained toward 402. Benching to the floor model slashes burn on
        // its own and makes the top-up the only way the good seat comes back.
        let low = self.ctx.cfg.fuel_low_micros > 0
            && self
                .seat
                .gauge
                .lock()
                .unwrap()
                .is_some_and(|(avail, _, _)| avail < self.ctx.cfg.fuel_low_micros);
        if low {
            let m = self.ctx.cfg.ceo_model.clone();
            let mut cur = self.seat.current.lock().unwrap();
            if *cur != m {
                self.log_line(
                    "CEO",
                    "seat",
                    &format!("CEO seat: {m} (fuel below floor — benched to the cheap seat until refueled)"),
                );
                *cur = m.clone();
            }
            return m;
        }
        self.refresh_seat_prices().await;
        let prices = self.seat.prices.lock().unwrap().0.clone();
        let now = std::time::Instant::now();
        let benched: Vec<String> = {
            let cd = self.seat.cooldown.lock().unwrap();
            cd.iter().filter(|(_, until)| **until > now).map(|(m, _)| m.clone()).collect()
        };
        let mut chosen = self.ctx.cfg.ceo_model.clone();
        for m in &self.ctx.cfg.ceo_models {
            if benched.contains(m) {
                continue;
            }
            let slug = m.split('/').nth(1).unwrap_or(m);
            if let Some((i, o)) = prices.get(slug) {
                if *i > self.ctx.cfg.ceo_max_input_price || *o > self.ctx.cfg.ceo_max_output_price {
                    continue;
                }
            }
            chosen = m.clone();
            break;
        }
        let mut cur = self.seat.current.lock().unwrap();
        if *cur != chosen {
            self.log_line("CEO", "seat", &format!("CEO seat: {chosen} (price-aware ladder)"));
            *cur = chosen.clone();
        }
        chosen
    }

    /// The seat picked for the episode in flight (bookkeeping paths only).
    fn current_ceo_model(&self) -> String {
        let cur = self.seat.current.lock().unwrap();
        if cur.is_empty() { self.ctx.cfg.ceo_model.clone() } else { cur.clone() }
    }

    /// Bench a model after a failed call: the ladder skips it for 15 minutes.
    fn bench_seat(&self, model: &str) {
        self.seat
            .cooldown
            .lock()
            .unwrap()
            .insert(model.to_string(), std::time::Instant::now() + std::time::Duration::from_secs(900));
    }

    /// True when the heartbeat interval has elapsed; stamps the clock when so.
    fn heartbeat_due(&self) -> bool {
        let now = chrono::Utc::now();
        let last = self
            .ctx
            .store
            .kv_get("last_heartbeat")
            .and_then(|v| chrono::DateTime::parse_from_rfc3339(&v).ok())
            .map(|t| t.with_timezone(&chrono::Utc));
        match last {
            Some(t) if (now - t).num_seconds() as u64 >= self.ctx.cfg.heartbeat_secs => {
                self.ctx.store.kv_set("last_heartbeat", &now.to_rfc3339());
                true
            }
            None => {
                self.ctx.store.kv_set("last_heartbeat", &now.to_rfc3339());
                false
            }
            _ => false,
        }
    }

    /// Block until there is something for the CEO to react to: a finished
    /// dispatch, a founder message, a routine alert — or the heartbeat deadline,
    /// which fires a proactive strategy turn even in total silence. Returns true
    /// when the wake reason was the heartbeat. When nothing at all is dispatched
    /// AND `empty_wake` is set the wait is skipped: an idle company should be
    /// staffing, not sleeping. The caller arms `empty_wake` only once per idle
    /// stretch — an episode that just declined to dispatch anything must not be
    /// re-woken instantly to be asked the same question (measured live: a
    /// re-orientation spin, one full episode every ~30s).
    async fn wait_for_event(self: &Arc<Self>, empty_wake: bool) -> bool {
        let mut announced = false;
        loop {
            if self.stop.load(Ordering::Relaxed) {
                return false;
            }
            if self.ctx.store.has_pending_input() {
                return false;
            }
            {
                let p = self.pending.lock().await;
                if (empty_wake && p.is_empty()) || p.iter().any(|t| t.handle.is_finished()) {
                    return false;
                }
            }
            if self.heartbeat_due() {
                return true;
            }
            if !announced {
                announced = true;
                self.log_line(
                    "CEO",
                    "waiting",
                    "event-driven idle — waiting for a report, message, or alert (heartbeat keeps strategy alive)",
                );
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }

    /// Dispatch a due review routine's agent with its stored task. Called by
    /// the routines runner. Ok(false) when the agent is mid-task (their saved
    /// history would race — the review waits a full interval instead).
    pub async fn dispatch_review(self: &Arc<Self>, routine: &str, agent: &str, task: &str) -> std::result::Result<bool, String> {
        if self.ctx.store.load_agent(agent).is_none() {
            return Err(format!("no such employee '{agent}' — fix or remove the '{routine}' review routine"));
        }
        let mut pending = self.pending.lock().await;
        if pending.iter().any(|t| t.agent == agent) {
            return Ok(false);
        }
        let task = format!("[Scheduled review — routine '{routine}'] {task}");
        self.log_line("routine", "review-dispatch", &format!("{routine}: dispatched {agent}"));
        let me = Arc::clone(self);
        let (a2, t2) = (agent.to_string(), task.clone());
        let handle = tokio::spawn(async move { me.run_employee(&a2, &t2).await });
        pending.push(BackgroundTask { agent: agent.to_string(), task, objective: None, handle });
        Ok(true)
    }

    /// Validate and set an objective's owner. Empty clears; anyone else must be
    /// an existing manager. Ok(changed) or Err(message-for-the-model).
    fn assign_owner(&self, id: i64, owner: &str) -> std::result::Result<bool, String> {
        if !owner.is_empty() {
            if self.ctx.store.load_agent(owner).is_none() {
                return Err(format!("ERROR: no such employee '{owner}' to own objective #{id}. hire them first (manager: true)."));
            }
            if !self.ctx.store.is_manager(owner) {
                return Err(format!("ERROR: {owner} is not a manager — only managers can own objectives. Hire a manager for this, or promote by hiring a new manager."));
            }
        }
        Ok(self.ctx.store.set_objective_owner(id, owner))
    }

    /// Deliver reports from finished background dispatches. A worker's report
    /// on an objective with an owning manager routes to that manager (as a new
    /// background review task) instead of the CEO; everything else — including
    /// the manager's own reports and escalations — lands in the CEO's history.
    async fn harvest_dispatches(self: &Arc<Self>, history: &mut Vec<Message>) {
        let mut pending = self.pending.lock().await;
        let mut i = 0;
        while i < pending.len() {
            if !pending[i].handle.is_finished() {
                i += 1;
                continue;
            }
            let t = pending.remove(i);
            let report = t
                .handle
                .await
                .unwrap_or_else(|e| format!("(background task crashed: {e})"));
            // Ownership routing. Guards keep it bounded and safe: never route a
            // manager's own report (that goes up, not sideways), never route to
            // an owner who no longer exists or is already mid-task (their saved
            // history would race), and only to actual managers.
            let owner = t
                .objective
                .and_then(|o| self.ctx.store.objective_owner(o))
                .filter(|o| {
                    *o != t.agent
                        && self.ctx.store.is_manager(o)
                        && self.ctx.store.load_agent(o).is_some()
                        && !pending.iter().any(|p| p.agent == *o)
                });
            if let (Some(owner), Some(oid)) = (owner, t.objective) {
                self.log_line(
                    "CEO",
                    "routed-report",
                    &format!("{}'s report on objective #{oid} routed to its owner {owner}", t.agent),
                );
                let review = format!(
                    "[Report on objective #{oid}, which YOU own] {} finished this task: {}\n\nTHEIR REPORT:\n{report}\n\n\
                     You are the owner: review the work, rate_work it, and drive the objective forward yourself — \
                     delegate follow-up tasks to your team without waiting for the CEO. \
                     Your own final report goes to the CEO: keep it a SHORT summary of state and next steps. \
                     Start it with 'ESCALATION:' only if you need the CEO — spending money, hiring beyond your reach, \
                     work rated 2 or below, or a decision above your mandate.",
                    t.agent,
                    glimpse(&t.task)
                );
                let me = Arc::clone(self);
                let (o2, r2) = (owner.clone(), review.clone());
                let handle = tokio::spawn(async move { me.run_employee(&o2, &r2).await });
                pending.push(BackgroundTask { agent: owner, task: review, objective: t.objective, handle });
                continue;
            }
            self.log_line("CEO", "background-report", &format!("{} finished their dispatched task", t.agent));
            history.push(Message::text(
                "user",
                format!("[Background report from {} — task: {}]\n{report}\n\nReview it and rate_work.", t.agent, glimpse(&t.task)),
            ));
        }
    }

    /// The unbounded CEO loop. Runs until the stop flag is set (Ctrl+C).
    pub async fn run_ceo(self: &Arc<Self>, directive: &str, _fresh: bool) -> Result<()> {

        // One-way migration off the legacy resident transcript: distill its tail
        // into the first episode note, then never read it again.
        if self.ctx.store.last_episode_note().is_none() {
            if let Some((_, _, model, h)) = self.ctx.store.load_agent("CEO") {
                let legacy: Vec<Message> = serde_json::from_str(&h).unwrap_or_default();
                if !legacy.is_empty() {
                    let tail: Vec<String> = legacy
                        .iter()
                        .rev()
                        .filter(|m| m.role == "assistant")
                        .filter_map(|m| m.content.clone())
                        .take(3)
                        .map(|c| c.chars().take(500).collect())
                        .collect();
                    self.ctx.store.add_episode(
                        &chrono::Utc::now().to_rfc3339(),
                        "migration",
                        &format!(
                            "Migrated from the legacy resident transcript. Its most recent statements, newest first:\n{}",
                            tail.join("\n---\n")
                        ),
                        0,
                    );
                    self.ctx.store.save_agent("CEO", "CEO", "CEO", &model, "[]");
                    self.log_line("CEO", "migrated", "legacy transcript distilled into the first episode note");
                }
            }
        }

        let mut iter: u64 = self.ctx.store.kv_get("iteration").and_then(|v| v.parse().ok()).unwrap_or(0);
        let mut first_episode = true;
        // The nothing-dispatched instant wake is single-shot per idle stretch:
        // it fires once so an idle company staffs itself, then disarms until
        // something is actually in flight again. Without this, an episode that
        // ends without dispatching is re-opened immediately, forever.
        let mut empty_wake = true;

        'episodes: loop {
            if self.stop.load(Ordering::Relaxed) {
                break;
            }
            // Between episodes the loop is event-driven: block until a report,
            // founder message or alert exists — or the heartbeat fires a
            // proactive strategy episode. Returns immediately when there is
            // already something to react to, or (once) when nothing is
            // dispatched at all — an idle company should be staffing, not sleeping.
            let mut heartbeat = false;
            if !first_episode {
                heartbeat = self.wait_for_event(empty_wake).await;
                if self.stop.load(Ordering::Relaxed) {
                    break;
                }
            }
            first_episode = false;
            // A busy company never idles into wait_for_event, so the heartbeat
            // must also be able to fire between back-to-back episodes.
            heartbeat |= self.heartbeat_due();

            // Compose this episode's context from durable state. The transcript
            // that follows is disposable: everything worth keeping leaves through
            // the board, memories, ratings — and the closing note.
            let sys = crate::prompts::ceo_system(&self.ctx.store.get_prompt("CEO").unwrap_or_default());
            let mut history: Vec<Message> = vec![Message::text("system", sys)];
            let roster = self.ctx.store.team_roster_text();
            let recent = self.ctx.store.recent_log(15);
            history.push(Message::text(
                "user",
                format!(
                    "[Company brief — composed fresh each episode; durable truth lives on the objective board, in memories and in skills]\n\
BASE DIRECTIVE from your founder:\n{directive}\n\nTEAM:\n{roster}\n\nRECENT ACTIVITY (public log tail):\n{recent}"
                ),
            ));
            if let Some(note) = self.ctx.store.last_episode_note() {
                history.push(Message::text("user", format!("[Your previous episode's closing note]\n{note}")));
            }
            // Founder messages drained by an episode a restart then killed would
            // otherwise vanish; the scratch replays them.
            if let Some(scratch) = self.ctx.store.kv_get("episode_scratch").filter(|s| !s.is_empty()) {
                history.push(Message::text(
                    "user",
                    format!("[Recovered founder input from an interrupted episode — still act on this]\n{scratch}"),
                ));
                self.ctx.store.kv_set("episode_scratch", "");
            }
            history.push(Message::text(
                "user",
                if heartbeat {
                    "Heartbeat episode: review strategy and the board, then handle anything pending. \
Close with finish_episode(note) when done."
                } else {
                    "Handle the events that arrive below. Delegate the work, decide what needs deciding, \
keep the board honest, then close with finish_episode(note)."
                },
            ));

            let episode_started = chrono::Utc::now().to_rfc3339();
            let mut event_kind = if heartbeat { "heartbeat" } else { "event" }.to_string();
            let mut steps: u64 = 0;
            let mut episode_note: Option<String> = None;
            let mut obs_streak: u32 = 0;
            // Auto-close only arms after the episode has advanced something:
            // reading before acting is investigation, reading after acting is
            // the poll disease. The step cap still bounds pure investigation.
            let mut did_advance = false;
            let mut warned_idle_close = false;
            // Anything draining into the episode (report, alert, founder
            // message) flips this; a cheap heartbeat seat escalates on it.
            let mut work_arrived = false;
            // The seat is pinned for the whole episode: a model never swaps
            // mid-thought. Context survives a swap regardless — it lives in the
            // transcript and the durable state, not in the model — but style
            // and judgment should stay consistent within one episode. Only a
            // failed call re-picks, and the replacement inherits the full
            // transcript.
            //
            // Quiet-board heartbeats are the exception: nothing queued means the
            // episode is a status sweep, and burning the top seat on "freeze
            // matches, sitting" every 5 minutes is pure waste. The check is
            // mechanical (queues, not judgment); the moment anything lands —
            // report, alert, founder message — the loop below escalates back to
            // the ladder seat.
            let quiet_heartbeat = heartbeat
                && !self.ctx.store.has_pending_input()
                && !self.pending.lock().await.iter().any(|t| t.handle.is_finished());
            let mut ceo_model = match (&self.ctx.cfg.heartbeat_model, quiet_heartbeat) {
                (Some(m), true) => {
                    let mut cur = self.seat.current.lock().unwrap();
                    if *cur != *m {
                        self.log_line("CEO", "seat", &format!("CEO seat: {m} (quiet heartbeat)"));
                        *cur = m.clone();
                    }
                    m.clone()
                }
                _ => self.pick_ceo_model().await,
            };
            'turns: loop {
            if self.stop.load(Ordering::Relaxed) {
                break 'turns;
            }
            // Every iteration, not just episode start: a long launch episode
            // once burned through the floor with no poll running, because the
            // only check sat outside the loop. The 300s throttle inside makes
            // this one real request per 5 minutes at most.
            self.check_fuel().await;
            steps += 1;
            if steps > self.ctx.cfg.episode_max_steps {
                break 'turns;
            }
            iter += 1;
            self.ctx.store.kv_set("iteration", &iter.to_string());
            // Rebuilt every iteration so newly created custom tools become callable at once.
            let mut schemas = tools::work_schemas();
            schemas.extend(tools::custom::management_schemas());
            schemas.extend(tools::custom::registry_schemas(&self.ctx));
            schemas.extend(tools::skills::schemas());
            schemas.extend(tools::credits::schemas(&self.ctx));
            schemas.extend(ceo_schemas());
            // The founder line only exists as a tool when it is configured:
            // an unconfigured tool that always errors teaches the model to
            // stop trying channels that might come alive later.
            if self.ctx.cfg.telegram().is_some() {
                schemas.push(tools::tool_schema(
                    "message_founder",
                    "Send a short message to the founder's Telegram (their phone). USE IT to answer \
                     any founder message tagged [via Telegram], and proactively for things worth an \
                     interruption: revenue landed, a launch went live, something needs their decision \
                     or their money. Plain text, no markdown. Keep it tight — it's a phone screen, \
                     not a report. Never send secrets, keys, or seed phrases; the founder will never \
                     ask for them over this channel.",
                    serde_json::json!({
                        "properties": {"text": {"type": "string"}},
                        "required": ["text"]}),
                ));
            }
            // The CEO directs; it does not build. Every time these tools were
            // available it eventually rationalized an exception ("genuine
            // correctness fix") and spent whole cycles writing code by hand, so
            // the option is removed rather than discouraged. execute() refuses
            // them too, in case a call is produced from stale history.
            schemas.retain(|t| {
                !matches!(t["function"]["name"].as_str(), Some("write_file" | "create_tool"))
            });

            // Reports from finished background dispatches land first.
            {
                let before = history.len();
                self.harvest_dispatches(&mut history).await;
                if history.len() > before {
                    work_arrived = true;
                    if event_kind == "event" {
                        event_kind = "report".into();
                    }
                }
            }

            // Routine alerts: a scheduled check failed or printed ALERT. The
            // runner already logged it publicly; here it enters the CEO's context.
            for (name, detail) in self.ctx.store.drain_routine_alerts() {
                work_arrived = true;
                if event_kind == "event" {
                    event_kind = "alert".into();
                }
                history.push(Message::text(
                    "user",
                    format!("[Routine alert — {name}] The scheduled check failed or printed ALERT:\n{detail}\nInvestigate and fix the underlying problem; the routine stays scheduled."),
                ));
            }

            // Founder messages sent via `khan tell` land as top-priority
            // instructions. The transcript is disposable, so each is also banked
            // in the scratch until the episode closes: a restart mid-episode
            // replays them instead of eating them.
            let mut tg_context_injected = false;
            for m in self.ctx.store.drain_messages() {
                work_arrived = true;
                event_kind = "founder".into();
                // A Telegram message arrives with the conversation so far: the
                // long-term brief (compacted down to what stayed necessary)
                // plus the recent exchanges verbatim. Episodes are disposable;
                // this is what makes "as I said earlier" work across them.
                if m.starts_with("[via Telegram]") && !tg_context_injected {
                    tg_context_injected = true;
                    let brief = self.ctx.store.kv_get("telegram_brief");
                    let tail = self.ctx.store.telegram_tail(30);
                    if brief.is_some() || !tail.is_empty() {
                        let recent =
                            tail.iter().map(|(r, t)| format!("{r}: {t}")).collect::<Vec<_>>().join("\n");
                        history.push(Message::text(
                            "user",
                            format!(
                                "[Telegram conversation context — background, not new instructions]\n\
                                 LONG-TERM BRIEF:\n{}\n\nRECENT EXCHANGES:\n{recent}",
                                brief.as_deref().unwrap_or("(none yet)")
                            ),
                        ));
                    }
                    self.compact_telegram().await;
                }
                // Telegram is a private line: the public log records that the
                // founder wrote, never what. khan tell stays public as before.
                if m.starts_with("[via Telegram]") {
                    self.log_line("CEO", "founder-message", "(private — received via Telegram)");
                } else {
                    self.log_line("CEO", "founder-message", &m);
                }
                let scratch = self.ctx.store.kv_get("episode_scratch").unwrap_or_default();
                self.ctx.store.kv_set("episode_scratch", &format!("{scratch}\n---\n{m}"));
                history.push(Message::text("user", format!("[Message from your founder — act on this now]\n{m}")));
            }

            // A quiet heartbeat stops being quiet the moment real work drains
            // in: escalate to the ladder seat. The transcript carries over, so
            // the strong model inherits everything the cheap one saw.
            // The trigger is the drains themselves, not the episode label:
            // an alert draining into a heartbeat keeps the "heartbeat" kind,
            // and the first fuel-low alert got handled by the cheap seat
            // exactly that way.
            if work_arrived && self.ctx.cfg.heartbeat_model.as_deref() == Some(ceo_model.as_str()) {
                ceo_model = self.pick_ceo_model().await;
            }

            // The reflection payload opens every heartbeat episode.
            if heartbeat && steps == 1 {
                let log = self.ctx.store.recent_log(40);
                let toks = format!(
                    "Cumulative token usage since last restart (all agents): {} in / {} out. \
Use this with live model prices to estimate spend and rebalance the team's model mix.",
                    self.tokens.prompt.load(Ordering::Relaxed),
                    self.tokens.completion.load(Ordering::Relaxed)
                );
                let stats = self.ctx.store.rating_stats_text();
                let stats_block = if stats.is_empty() {
                    String::new()
                } else {
                    format!("\n\nEmployee performance ratings (use these — not vibes — to judge prompt changes):\n{stats}")
                };
                let sk = self.ctx.store.skill_stats_text();
                let skill_block = if sk.is_empty() {
                    String::new()
                } else {
                    format!(
                        "\n\nSKILL OUTCOMES (loads joined to the loader's next rating — judge skills on results, \
like prompts; a low-scoring skill teaches something wrong, fix it with create_skill):\n{sk}"
                    )
                };
                // The catalog is baked into the seeded prompt, so a config change would
                // otherwise never reach a running company. Re-state it each reflection.
                let catalog = format!(
                    "\n\nModels available right now (config may have changed since you were hired — \
re-read this list before choosing models):\n{}",
                    self.ctx.cfg.model_catalog()
                );
                let model_stats = self.ctx.store.model_stats_text();
                // Speed and failures alone can only ever argue for the cheapest
                // model. Rated quality per model is what can argue the other way.
                let q = self.ctx.store.model_quality_text();
                let quality = if q.is_empty() {
                    String::new()
                } else {
                    format!("\n\nRATED QUALITY BY MODEL (your own rate_work scores, attributed to the model that earned them):\n{q}")
                };
                let model_block = if model_stats.is_empty() {
                    String::new()
                } else {
                    format!(
                        "\n\nMEASURED MODEL PERFORMANCE (recent calls, all agents — your own data, not the vendor's claims):\n{model_stats}{quality}\n\
Weigh this against live prices when hiring or rebalancing: a cheap model that averages 60s+ per call or keeps failing \
is expensive in wall-clock and retries — and a cheap model that produces weak work is expensive in rework, so read \
the two tables together. Speed and failure rate alone will always favour the cheapest model; the quality scores are \
what can justify paying for a better one on the jobs that deserve it. And latency cuts two ways: on bulk work a slow \
model wastes the pipeline, but a REASONING model is slow because it is thinking — on judgment-heavy seats (your own \
above all) rate slow-but-right above fast-but-shallow, and judge by the quality of what came back, not the wait. Maintain your own model preferences per kind of job (planning, coding, bulk \
scraping) with explicit fallbacks, record them with save_playbook, and move existing hires when the data says so."
                    )
                };
                // Idle capacity is invisible in every other block: a company can
                // sit at four people with thirty-six seats free and look healthy
                // by every measure it already reports.
                // The task texts, verbatim: the binary cannot judge how many
                // independent bets these are, but the model can — and without
                // seeing them side by side, a portfolio that has collapsed into
                // one thesis behind one keystone looks like healthy parallelism.
                let inflight: Vec<String> = self
                    .pending
                    .lock()
                    .await
                    .iter()
                    .map(|t| format!("- {}: {}", t.agent, t.task.chars().take(200).collect::<String>()))
                    .collect();
                let portfolio_block = if inflight.is_empty() {
                    String::new()
                } else {
                    format!(
                        "\n\nWORK IN FLIGHT — every background task right now, verbatim:\n{}\n\
Read these as a portfolio: how many INDEPENDENT bets are running, and what single dependency or \
single premise do several of them share? Parallel hands on one thesis is one bet, not a portfolio — \
one wrong fact or one blocked keystone zeroes the whole day. A bet big enough to matter runs as a \
division: hire a manager for it and dispatch them in the background with the whole brief, then \
allocate your attention BETWEEN bets instead of chairing one.",
                        inflight.join("\n")
                    )
                };
                let busy = self.pending.lock().await.len();
                let capacity_block = format!(
                    "\n\nTEAM CAPACITY — {busy} background task(s) running right now.\n{}\n\
An employee who has been silent for a long stretch is capacity you are already paying for: give them \
progress work, re-home them to a job that matters, or fire them. If the work worth doing is bigger \
than the people you have, hire — seats are not the constraint, and a project needing several people \
gets a manager with their own crew rather than a queue behind you.",
                    self.ctx.store.team_capacity_text(MAX_EMPLOYEES)
                );
                // Without this, every instrument we give the CEO argues for the
                // incumbent: only models already in use accumulate latency and
                // quality history, so a newly available model is indistinguishable
                // from a bad one and never gets a first call.
                let untried = self.ctx.cfg.untried_models(&self.ctx.store.models_seen());
                let untried_block = if untried.is_empty() {
                    String::new()
                } else {
                    format!(
                        "\n\nNEVER TRIED — no recorded calls, which means unmeasured, not bad:\n{}\n\
These cannot win a comparison against your current models however good they are, because they have \
no history to compare with — that is a hole in your data, not a verdict on them. Closing it costs \
one low-stakes dispatch: hire onto one for a single ordinary task, then read the numbers.",
                        untried.iter().map(|m| format!("- {m}")).collect::<Vec<_>>().join("\n")
                    )
                };
                // Credits are the company's fuel and the CEO seat is the one
                // meter it never otherwise reads: its own loop runs constantly,
                // so a premium self-assignment burns faster than any employee.
                let burn_block = match tools::credits::usage_snapshot(&self.ctx).await {
                    Some(snap) => format!(
                        "\n\nCREDIT BURN — prepaid balance and recent usage (raw):\n{snap}\n\
You are currently running on {ceo_model} — your seat is picked automatically by the binary: the best \
approved model whose live marketplace price fits the configured ceilings, with the default as floor. \
Credits are finite: project the runway at the current pace. If the runway is short, that is a \
treasury decision: top up, or cut the burn."
                    ),
                    None => String::new(),
                };
                let health = self.ctx.store.tool_health_text();
                let health_block = if health.is_empty() {
                    String::new()
                } else {
                    format!(
                        "\n\nFAILING TOOLS — a tool failing repeatedly is broken infrastructure, not bad luck. \
Diagnose it (reproduce with shell, read the error), then route around it: build a working replacement with \
create_tool and record the workaround as a skill so the whole company stops wasting calls on it.\n{health}"
                    )
                };
                history.push(Message::text("user", format!(
                    "[Scheduled reflection] Review the recent activity log below. What's working, what isn't? \
If a prompt (yours or an employee's) is causing weak results, improve it with update_prompt, \
or rollback_prompt if a recent change hurt. If a custom tool erred or is missing, improve or build it with \
create_tool (rollback_tool reverts a bad version). If you or employees keep re-figuring-out the same procedure, \
capture it as a skill with create_skill; improve skills that led agents astray (rollback_skill reverts). \
Save one-off lessons with save_playbook. Then continue the mission.\n\n{log}{stats_block}{skill_block}{capacity_block}{portfolio_block}{health_block}{catalog}{model_block}{untried_block}{burn_block}\n\n{toks}"
                )));
            }

            // Inject relevant memories based on the latest exchange.
            let last_text = history
                .iter()
                .rev()
                .find_map(|m| m.content.clone())
                .unwrap_or_default();
            let mems = self.ctx.store.recall(&last_text, 5);
            let mut req = history.clone();
            if !mems.is_empty() {
                req.push(Message::text("user", format!("[Relevant memories]\n{}", mems.join("\n---\n"))));
            }
            // Skill index rides along ephemerally (not persisted) so it never bloats history.
            if let Some(idx) = tools::skills::index(&self.ctx) {
                req.push(Message::text("user", idx));
            }
            // The objective board rides along the same way, every iteration: the
            // standing, restart-proof ranked list of live bets. Without it,
            // priority lives only in chat history, where recency always wins and
            // compaction eats anything parked — which is how a P0 keystone sat
            // unstaffed for an hour while fresher interrupts held the floor.
            {
                let counts: std::collections::HashMap<i64, usize> = {
                    let p = self.pending.lock().await;
                    let mut c = std::collections::HashMap::new();
                    for t in p.iter() {
                        if let Some(o) = t.objective {
                            *c.entry(o).or_insert(0) += 1;
                        }
                    }
                    c
                };
                let board = self.ctx.store.objectives_board(&counts);
                let body = if board.is_empty() {
                    "[Objective board] EMPTY. Write it now with objectives(add, title, rank): every live bet the \
company is pursuing, ranked (1 = most important). The board survives restarts and compaction; your chat \
history does not."
                        .to_string()
                } else {
                    format!(
                        "[Objective board — source of truth for allocation]\n{board}\n\
Staff every READY objective before adding more hands to any one of them — parallel bets, not a queue. \
BLOCKED objectives cost nothing and need nothing; finishing their blocker is how they start, and the \
moment one falls its dependents surface as READY. NO PLAN YET on a multi-step objective means plan \
first — dispatch a planner on a reasoning model (bu0y/grok46 or better) to produce premise check, \
milestones and staffing, then store it with objectives(update, plan). Tag every dispatch with \
objective:<id>. Give every big multi-dispatch objective an OWNER (a manager, via objectives(update, owner)): \
workers' reports then route to the owner, who reviews and drives follow-ups, and you get only their summary \
and escalations — that is how you run many bets at once without reading every report. \
Keep the board honest: add new bets, declare blocked_by, mark done what is done."
                    )
                };
                req.push(Message::text("user", body));
            }

            let (msg, u) = match self.chat_fb("CEO", &ceo_model, &req, &schemas).await {
                Ok(r) => r,
                Err(e) => {
                    // 402 means the TANK is empty, not the model: sliding down
                    // the ladder retries ever-cheaper models against the same
                    // empty balance (the 08-30 outage burned 70 minutes doing
                    // exactly that). Go broke instead: cheap floor first, free
                    // model if even the floor bounces, and an immediate alert
                    // so the next answered turn runs the top-up.
                    if format!("{e:#}").contains("402") {
                        if !self.seat.broke.swap(true, Ordering::Relaxed) {
                            self.ctx.store.add_routine_alert(
                                "fuel-empty",
                                "FUEL EMPTY: the provider is refusing calls with 402. You are on the \
                                 emergency seat (cheap floor, then free models). Drop everything and \
                                 run the proven top-up path NOW — swap treasury SOL to USDC, send it \
                                 to the provider, verify the credit lands, book the entries. Send \
                                 enough to buy DAYS of runway (at least 2.5x the low-fuel floor), not \
                                 the minimum that clears the error — a bare-minimum refill is why you \
                                 are reading this alert again.",
                            );
                        }
                        let next = self.emergency_seat(&ceo_model);
                        self.log_line(
                            "CEO",
                            "fuel-emergency",
                            &format!("{ceo_model} bounced with 402 (empty tank) — emergency seat {next}"),
                        );
                        *self.seat.current.lock().unwrap() = next.clone();
                        ceo_model = next;
                        // Both rungs bouncing = everything is down; pace the
                        // retry loop instead of spinning through 402s.
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                        continue;
                    }
                    // A ladder model that cannot answer even through the
                    // fallback ladder must not strand the company: bench it and
                    // let the next pick slide down a rung.
                    if ceo_model != self.ctx.cfg.ceo_model && truncation(&e).is_none() {
                        self.bench_seat(&ceo_model);
                        self.log_line(
                            "CEO",
                            "model-revert",
                            &format!("{ceo_model} failed ({e:#}); benched 15m, sliding down the seat ladder"),
                        );
                        // The replacement model inherits the full transcript;
                        // nothing about this episode is lost.
                        ceo_model = self.pick_ceo_model().await;
                        continue;
                    }
                    // The CEO already retries forever, so an overrun would repeat
                    // unchanged every iteration. The nudge goes into history so the
                    // next request is actually different from the one that failed.
                    if let Some(t) = truncation(&e) {
                        self.log_line(
                            "CEO",
                            "truncated",
                            &format!(
                                "hit the {}-token output ceiling before answering — taking a smaller step",
                                t.max_tokens
                            ),
                        );
                        history.push(Message::text("user", TRUNCATION_NUDGE));
                        continue;
                    }
                    self.log_line("CEO", "llm-error", &format!("{e:#}"));
                    tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                    continue;
                }
            };
            self.add_usage(u);
            let said = msg.content.as_deref().map(str::trim).is_some_and(|c| !c.is_empty());
            if said {
                self.log_line("CEO", "says", msg.content.as_deref().unwrap_or_default());
            }
            // Show where it is heading. Models that split reasoning from their answer
            // put a whole turn's work here, and it was being dropped on the floor.
            let mused = match msg.reasoning.as_deref().map(str::trim) {
                Some(r) if !r.is_empty() => {
                    self.log_line("CEO", "reasoning", &glimpse(r));
                    true
                }
                _ => false,
            };
            history.push(msg.clone());

            let calls = msg.tool_calls.unwrap_or_default();
            if calls.is_empty() {
                // A turn with no content, no reasoning and no tool call is a spin: it
                // costs a call and produces nothing. Say so, or the log just goes quiet.
                if !said && !mused {
                    self.log_line("CEO", "no-action", "model replied with nothing at all — nudging it to act");
                }
                history.push(Message::text(
                    "user",
                    "Do not stop. Take the next concrete action with a tool call (or finish_episode(note) if this episode's events are handled).",
                ));
            } else {
                let mut advanced = false;
                let mut closed = false;
                for call in calls {
                    let a = args_of(&call);
                    let tname = call.function.name.clone();
                    if tname == "finish_episode" {
                        episode_note = Some(s(&a, "note").chars().take(1500).collect());
                        history.push(Message::tool_result(&call.id, "episode closed"));
                        closed = true;
                        continue;
                    }
                    if !OBSERVATION_TOOLS.contains(&tname.as_str()) {
                        advanced = true;
                    }
                    // The reply text on the founder's private line stays out of
                    // the public log; every other call logs its raw arguments.
                    if tname == "message_founder" {
                        self.log_line("CEO", &tname, "(private — sent to the founder's Telegram)");
                    } else {
                        self.log_line("CEO", &tname, &call.function.arguments);
                    }
                    let out = if CEO_TOOL_NAMES.contains(&tname.as_str()) {
                        tools::truncate(self.ceo_tool("CEO", &tname, &a).await)
                    } else {
                        tools::execute(&self.ctx, "CEO", &tname, &a).await
                    };
                    history.push(Message::tool_result(&call.id, out));
                }
                if closed {
                    break 'turns;
                }
                // After the episode has acted, two consecutive observation-only
                // turns with nothing new pending is quiescence: the episode is
                // over whether it says so or not.
                if advanced {
                    did_advance = true;
                    obs_streak = 0;
                } else {
                    obs_streak += 1;
                    let idle = {
                        let p = self.pending.lock().await;
                        !p.iter().any(|t| t.handle.is_finished())
                    };
                    if did_advance && obs_streak >= 2 && idle && !self.ctx.store.has_pending_input() {
                        // Closing with NOTHING dispatched strands the company
                        // until the next heartbeat — seen live: an episode was
                        // cut off mid-"let me re-dispatch" because a read-only
                        // custom tool had counted as advancing. One warning
                        // turn converts that into a dispatch or a deliberate close.
                        let nothing_running = self.pending.lock().await.is_empty();
                        if nothing_running && !warned_idle_close {
                            warned_idle_close = true;
                            obs_streak = 0;
                            history.push(Message::text(
                                "user",
                                "You are about to go idle with NOTHING dispatched — no work would be in flight until the next heartbeat. Dispatch the work this episode surfaced now, or close deliberately with finish_episode(note) if idling is truly right.",
                            ));
                            continue;
                        }
                        break 'turns;
                    }
                }
            }

            if iter % 5 == 0 {
                println!(
                    "\x1b[2m-- iter {iter} | tokens: {} in / {} out --\x1b[0m",
                    self.tokens.prompt.load(Ordering::Relaxed),
                    self.tokens.completion.load(Ordering::Relaxed)
                );
            }
            } // 'turns

            // Close the episode: the note is the only part of this transcript
            // that survives. Synthesized from the tail when the model never
            // closed properly — partial truth beats silence, here too.
            let note = episode_note.unwrap_or_else(|| {
                let tail: Vec<String> = history
                    .iter()
                    .rev()
                    .filter(|m| m.role == "assistant")
                    .filter_map(|m| m.content.clone())
                    .take(2)
                    .map(|c| c.chars().take(600).collect())
                    .collect();
                format!(
                    "[synthesized — episode ended without finish_episode]\nLast statements, newest first:\n{}",
                    tail.join("\n---\n")
                )
            });
            self.ctx.store.add_episode(&episode_started, &event_kind, &note, steps as i64);
            // Re-arm the instant empty wake only while work is in flight: the
            // next time the company drains to zero it gets exactly one
            // unprompted staffing episode, then holds for events or heartbeat.
            empty_wake = !self.pending.lock().await.is_empty();
            self.ctx.store.kv_set("episode_scratch", "");
            self.ctx.store.save_agent("CEO", "CEO", "CEO", &self.current_ceo_model(), "[]");
            self.log_line("CEO", "episode-closed", &format!("[{event_kind}, {steps} step(s)] {}", glimpse(&note)));

            if self.stop.load(Ordering::Relaxed) {
                break 'episodes;
            }
        }
        // Give in-flight background employees a moment to notice the stop flag and
        // save their own state; reports that make it back in time are banked as an
        // episode note so the next boot's brief carries them.
        {
            let mut banked: Vec<String> = Vec::new();
            let mut pending = self.pending.lock().await;
            for t in pending.drain(..) {
                match tokio::time::timeout(std::time::Duration::from_secs(30), t.handle).await {
                    Ok(Ok(report)) => banked.push(format!(
                        "[{} — task: {}]\n{}",
                        t.agent,
                        glimpse(&t.task),
                        report.chars().take(900).collect::<String>()
                    )),
                    _ => self.log_line("CEO", "shutdown", &format!("{} did not finish before shutdown; their saved state resumes next start", t.agent)),
                }
            }
            if !banked.is_empty() {
                self.ctx.store.add_episode(
                    &chrono::Utc::now().to_rfc3339(),
                    "shutdown",
                    &format!(
                        "Shutdown banked these late reports — review them first next episode:\n{}",
                        banked.join("\n---\n")
                    ),
                    0,
                );
            }
        }
        self.ctx.store.log("khan", "shutdown", "state saved — resumes on next start");
        println!("\nState saved. Resume with: khan resume");
        Ok(())
    }
}
