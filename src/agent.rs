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
        tool("remove_routine", "Delete a scheduled routine.", json!({"name": {"type": "string"}}), json!(["name"])),
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
        tool("save_playbook", "Save a durable lesson/playbook entry that will be recalled in future relevant work.", json!({
            "topic": {"type": "string"}, "content": {"type": "string"}}),
            json!(["topic", "content"])),
        tool("finish", "Record a milestone report for the founder. Work continues afterwards.", json!({
            "report": {"type": "string"}}), json!(["report"])),
        tool("finish_episode", "Close this working episode. Your transcript is DISPOSABLE — only what you write here, on the board, and in memories survives to the next episode. The note is your handoff to your next self: what changed, what is in flight and with whom, and what the next episode must do or know. Call this when the events you woke for are handled and everything else is delegated or on the board.", json!({
            "note": {"type": "string", "description": "What changed, what is in flight, what the next episode must know. Max ~1500 chars."}}),
            json!(["note"])),
        tool("set_ceo_model", "Switch which model YOU (the CEO) run on, starting next iteration. Choose from the approved pool (call with model '?' to list it, with quality scores where known). Match the model to the stakes: your seat makes every hiring, treasury and strategy call, so a stronger model here pays for itself — but if your chosen model starts failing you are reverted to the reliable default automatically.", json!({
            "model": {"type": "string", "description": "provider/model from the approved pool, or '?' to list the pool"}}),
            json!(["model"])),
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
    "add_routine", "remove_routine", "list_routines",
    "update_prompt", "rollback_prompt", "save_playbook", "finish", "set_ceo_model", "objectives",
    "finish_episode",
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
            "set_ceo_model" => {
                if caller != "CEO" {
                    return "ERROR: only the CEO can change the CEO's model".into();
                }
                let pool = self.ceo_pool();
                let m = s(a, "model");
                if pool.iter().any(|p| p == m) {
                    self.ctx.store.kv_set("ceo_model", m);
                    self.log_line("CEO", "model-change", &format!("CEO moving to {m}"));
                    format!("done — you run on {m} from the next iteration. If it starts failing you revert to {} automatically.", self.ctx.cfg.ceo_model)
                } else {
                    format!(
                        "approved pool (default first, it is the fail-safe floor):\n{}\ncurrently on: {}",
                        pool.join("\n"),
                        self.current_ceo_model()
                    )
                }
            }
            _ => format!("unknown tool {name}"),
        }
    }

    /// Models the CEO may run itself on: the configured floor plus the vetted pool.
    fn ceo_pool(&self) -> Vec<String> {
        let mut pool = vec![self.ctx.cfg.ceo_model.clone()];
        for m in &self.ctx.cfg.ceo_models {
            if !pool.contains(m) {
                pool.push(m.clone());
            }
        }
        pool
    }

    /// The model the CEO runs on right now: its own persisted choice when that
    /// is still in the approved pool, else the configured floor.
    fn current_ceo_model(&self) -> String {
        self.ctx
            .store
            .kv_get("ceo_model")
            .filter(|m| self.ceo_pool().contains(m))
            .unwrap_or_else(|| self.ctx.cfg.ceo_model.clone())
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

            'turns: loop {
            if self.stop.load(Ordering::Relaxed) {
                break 'turns;
            }
            steps += 1;
            if steps > self.ctx.cfg.episode_max_steps {
                break 'turns;
            }
            iter += 1;
            self.ctx.store.kv_set("iteration", &iter.to_string());
            let ceo_model = self.current_ceo_model();
            // Rebuilt every iteration so newly created custom tools become callable at once.
            let mut schemas = tools::work_schemas();
            schemas.extend(tools::custom::management_schemas());
            schemas.extend(tools::custom::registry_schemas(&self.ctx));
            schemas.extend(tools::skills::schemas());
            schemas.extend(tools::credits::schemas(&self.ctx));
            schemas.extend(ceo_schemas());
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
                if history.len() > before && event_kind == "event" {
                    event_kind = "report".into();
                }
            }

            // Routine alerts: a scheduled check failed or printed ALERT. The
            // runner already logged it publicly; here it enters the CEO's context.
            for (name, detail) in self.ctx.store.drain_routine_alerts() {
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
            for m in self.ctx.store.drain_messages() {
                event_kind = "founder".into();
                self.log_line("CEO", "founder-message", &m);
                let scratch = self.ctx.store.kv_get("episode_scratch").unwrap_or_default();
                self.ctx.store.kv_set("episode_scratch", &format!("{scratch}\n---\n{m}"));
                history.push(Message::text("user", format!("[Message from your founder — act on this now]\n{m}")));
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
You are currently running on {ceo_model}. Credits are finite: project the runway at the current pace, \
and treat a premium model on your own seat as a stretch, not a residence — your loop runs every few \
seconds all day, and most iterations are routine orchestration that the default handles fine. Drop \
back with set_ceo_model when the work in front of you is routine; step up when real decisions are on \
the table. Switching is free and instant in both directions. If the runway is short, that is a \
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
Save one-off lessons with save_playbook. Then continue the mission.\n\n{log}{stats_block}{capacity_block}{portfolio_block}{health_block}{catalog}{model_block}{untried_block}{burn_block}\n\n{toks}"
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
                    // A self-chosen model that cannot answer even through the
                    // fallback ladder must not strand the company: revert to the
                    // configured floor and say so, rather than looping on it.
                    if ceo_model != self.ctx.cfg.ceo_model && truncation(&e).is_none() {
                        self.ctx.store.kv_set("ceo_model", &self.ctx.cfg.ceo_model);
                        self.log_line(
                            "CEO",
                            "model-revert",
                            &format!("{ceo_model} failed ({e:#}); reverting the CEO to {}", self.ctx.cfg.ceo_model),
                        );
                        history.push(Message::text(
                            "user",
                            &format!(
                                "[System] Your chosen model {ceo_model} failed and you are back on {}. \
                                 Pick a different pool model with set_ceo_model, or stay here.",
                                self.ctx.cfg.ceo_model
                            ),
                        ));
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
                    self.log_line("CEO", &tname, &call.function.arguments);
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
