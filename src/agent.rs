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
            "objective": {"type": "integer", "description": "REQUIRED: the board objective this task advances, or 0 for company upkeep (bookkeeping, infra, hygiene). The board shows each objective's 24h build/check mix from these tags."}}),
            json!(["agent", "task", "objective"])),
        tool("objectives", "Maintain the OBJECTIVE BOARD — the standing, restart-proof list of every live bet, ranked. It is shown to you every iteration with in-flight counts and staleness, and it is the source of truth for allocation. Actions: add (title, rank — lower rank = more important, 1 is P0), update (id + any of title/rank/plan/note/blocked_by), done (id), drop (id). Store each objective's plan with update once a planner has produced one. Declare dependencies honestly with blocked_by — blocked objectives are exempt from staffing pressure, and completing a blocker automatically surfaces its dependents as READY.", json!({
            "action": {"type": "string", "enum": ["add", "update", "done", "drop"]},
            "id": {"type": "integer"},
            "title": {"type": "string"},
            "rank": {"type": "integer", "description": "1 = most important. Rank every live bet honestly; ties are fine."},
            "plan": {"type": "string", "description": "The current plan: premise check, milestones, staffing. Written by a planning dispatch on a reasoning model, stored here."},
            "note": {"type": "string", "description": "One-line status note shown on the board"},
            "blocked_by": {"type": "string", "description": "Comma-separated objective ids this waits on (e.g. '3' or '2,3'); empty string clears. Work that needs an account or artifact another objective produces is BLOCKED, not hard."},
            "owner": {"type": "string", "description": "Manager who OWNS this objective; empty string clears. Workers' reports on an owned objective route to the owner, who reviews, rates and drives follow-up work — you get their summary and escalations only. Give every big objective an owner so your attention stays on allocation."},
            "kind": {"type": "string", "enum": ["profit", "growth", "infra", "explore"], "description": "Portfolio category — every objective needs one, and the weekly portfolio review judges each by its own yardstick. profit: exists to earn (launches, fees, trading) — judged revenue vs cost. growth: buys audience (social presence, listings, content) — judged cost per attention and trend, NEVER on revenue. infra: keeps the company running (automation, bookkeeping, site plumbing) — judged reliability and cost trend. explore: buys knowledge (premise checks, probes) — judged learning per capped dollar."}}),
            json!(["action"])),
        tool("team_status", "List background tasks started with dispatch: who is still working and on what.", json!({}), json!([])),
        tool("add_routine", "Schedule a shell command the binary runs itself, forever, at zero model cost. Silent when it passes; if it exits nonzero, times out, or prints ALERT, the alert is DISPATCHED to the routine's owner (set one — an alert is the domain owner's work, not yours); with no owner it lands in your inbox. Any check you have performed the same way roughly three times belongs here — verification scripts, health checks, reconciliation. Same name = replace.", json!({
            "name": {"type": "string", "description": "Short unique name, e.g. 'claim-cycle-verify'"},
            "command": {"type": "string", "description": "Shell command, run from the workspace. Print ALERT plus details to flag a problem; print nothing special when healthy."},
            "interval_secs": {"type": "integer", "description": "Seconds between runs, minimum 60"},
            "owner": {"type": "string", "description": "Existing employee who owns this domain — its alerts dispatch to them with the alert text, and their report routes back normally. Empty = alerts wake you instead."},
            "purpose": {"type": "string", "description": "One line on what deviation this catches"}}),
            json!(["name", "command", "interval_secs"])),
        tool("own_routine", "Assign an existing routine's alerts to an owning employee (empty owner = back to your inbox). An alert is the domain owner's work: routed alerts dispatch the owner directly and never interrupt you.", json!({
            "name": {"type": "string", "description": "The routine's name"},
            "owner": {"type": "string", "description": "Existing employee to own its alerts, or empty to clear"}}),
            json!(["name"])),
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
        tool("ack_founder", "Mark a founder directive DONE. Every `khan tell` message stays in your brief under OPEN FOUNDER DIRECTIVES, episode after episode, until you call this with its id — reading it is not doing it. Ack only when the thing asked for has actually happened (the skill version exists, the dispatch fired, the decision is on the board); an unfinished directive that falls out of the brief is how founder instructions used to get lost at the episode cut-off.", json!({
            "id": {"type": "integer", "description": "The directive's id as shown in the brief (#id)"},
            "note": {"type": "string", "description": "One line: what was done to satisfy it"}}),
            json!(["id"])),
    ]
}

/// The standing block of founder directives for the brief — empty when none
/// are open. A `khan tell` used to be a one-time event: the CEO read the
/// 23:44Z x_api_ops fold request, said "must land in the skill", hit the
/// episode cut-off looking for a file, and the next heartbeat had no memory
/// of it. Ceiling: the ten newest are shown in full, older ones counted.
pub(crate) fn open_directives_text(open: &[(i64, String, String)]) -> String {
    if open.is_empty() {
        return String::new();
    }
    const SHOW: usize = 10;
    let mut out = String::from(
        "\n\nOPEN FOUNDER DIRECTIVES (delivered, NOT yet acknowledged — each stays here every episode until you ack_founder(id) once it is actually done; reading is not doing):",
    );
    let skip = open.len().saturating_sub(SHOW);
    if skip > 0 {
        out.push_str(&format!("\n({skip} older still open — ack or act on those too)"));
    }
    for (id, ts, msg) in &open[skip..] {
        let text: String = msg.chars().take(600).collect();
        let more = if msg.chars().count() > 600 { "…" } else { "" };
        out.push_str(&format!("\n#{id} [{}]: {text}{more}", ts.get(..16).unwrap_or(ts)));
    }
    out
}

/// The kernel's fuel reading for the brief — empty until the first poll. The
/// CEO and CFO built their own "fuel window" doctrine on top of the kernel's
/// alert and topped up three times on 2026-09-01 with the tank above $100
/// and six days of runway; this states the rule the send tools enforce.
pub(crate) fn fuel_brief_line(store: &crate::state::Store) -> String {
    let Some(avail) = store.kv_get("fuel_available_micros").and_then(|v| v.parse::<u64>().ok()) else {
        return String::new();
    };
    let target = store.kv_get("fuel_refill_target_micros").and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    let burn = store.kv_get("fuel_burn_micros_per_hour").and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    let days = if burn > 0 { format!("{:.1} days at the measured burn", avail as f64 / (burn as f64 * 24.0)) } else { "burn not yet measured".into() };
    format!(
        "\n\nFUEL (kernel reading): ${:.2} in the tank — {days}. The kernel alerts you when a top-up is due and sizes it to reach ${:.2}; while the tank is above that target a send to the provider's deposit address is REFUSED by the send tools. Do not plan, stage, or schedule top-ups yourself — no runway windows, no deadlines, no treasury-gated triggers.",
        avail as f64 / 1e6,
        target as f64 / 1e6
    )
}

/// The brief's standing debt: ideas the company gave a review date that has
/// come and gone. Scanning is on a routine and a review routine; converting was
/// on nobody's calendar, so 16 premise rows and 13 candidates accumulated and
/// the only writes they got were appended notes. A date the company set for
/// itself is a promise, and this line keeps it in front of the CEO until the
/// row moves.
pub(crate) fn overdue_ideas_line(workspace: &std::path::Path) -> String {
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let rows = crate::tools::sql::overdue_ideas(workspace, &today);
    if rows.is_empty() {
        return String::new();
    }
    let shown = rows.iter().take(8).map(|(id, name, status, due)| {
        format!("\n  - id{id} {name} ({status}, review date {due})")
    }).collect::<String>();
    let more = if rows.len() > 8 { format!("\n  ...and {} more.", rows.len() - 8) } else { String::new() };
    format!(
        "\n\nIDEAS PAST THEIR OWN REVIEW DATE ({}):{shown}{more}\nThe company set these dates, and they have passed. Each is a decision you owe now: \
         hand it to an execution lane with a named owner, kill it with the number that killed it, or write the ONE missing fact and the date you will have it. \
         Appending a note to the row is not a decision, and another scan cycle does not answer for them.",
        rows.len()
    )
}

/// Output ceiling for the summarisation calls (history compaction, dropped
/// messages). Generous for a brief, and small enough that the gateway reserves
/// against a real number instead of the model's whole 64k ceiling.
pub(crate) const SUMMARY_MAX_TOKENS: u32 = 8192;
/// Window for a model's realized price. Short enough to follow a repricing
/// within the hour, long enough to hold the five fills the price needs.
const PEER_PRICE_HOURS: i64 = 3;
/// A peer answering fewer than this share of its recent calls is not a seat,
/// whatever it costs: with price caps on, a cheap band that cannot take our
/// concurrency shows up here as 503s, and moving agents onto it would park
/// them in waits. Unknown (too few calls) counts as fit, so a quiet peer
/// still gets sampled and earns a rate.
pub(crate) const PEER_MIN_OK_PCT: u64 = 80;

/// How long a stall stays on a model's record, and how many inside that window
/// bench it. Three in ten minutes is a route that is cutting answers off, not a
/// bad afternoon: on 2026-09-02 the failures came every three minutes.
pub(crate) const STALL_WINDOW: std::time::Duration = std::time::Duration::from_secs(600);
pub(crate) const STALL_STRIKES: usize = 3;

/// True when a failed call means the ROUTE stalled rather than the model
/// running out of its own budget. Either the gateway relayed an upstream
/// timeout, or the call burned two minutes before failing — at that point what
/// it says matters less than the two minutes the company spent waiting. A
/// speed-floor refusal is the same verdict delivered early: the gateway saying
/// no route to this model can keep up. Without the strike the CEO ladder
/// re-picked glm53flash every episode through 15:10–15:30Z on 2026-09-02 and
/// paid a refusal before every luna answer.
pub(crate) fn is_stall(err: &str, elapsed_secs: u64) -> bool {
    crate::llm::upstream_timeout(err) || err.contains("no route meets the speed floor") || elapsed_secs >= 120
}

/// Drop stalls that have aged out, record this one, and return how many now
/// stand against the model.
pub(crate) fn stall_strike(times: &mut Vec<std::time::Instant>, now: std::time::Instant) -> usize {
    times.retain(|t| now.duration_since(*t) < STALL_WINDOW);
    times.push(now);
    times.len()
}

/// base * 2^level, capped. Pure so the ladder is testable.
pub(crate) fn backoff_interval(base: u64, max: u64, level: u32) -> u64 {
    let mut v = base;
    for _ in 0..level.min(16) {
        v = v.saturating_mul(2);
        if v >= max {
            return max.max(base);
        }
    }
    v.min(max.max(base))
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

/// Authoritative tank line stitched above the raw /account/usage payload in the
/// reflection burn block. The raw body keeps a per-model "last error" that can
/// be DAYS stale — on 2026-08-31 a 41h-old 402 in it read as a live fuel
/// emergency and cost two episodes of re-diagnosis — so the kernel's own fresh
/// poll (check_fuel, ≤5 min old) must sit next to it and outrank it.
pub(crate) fn fuel_anchor(gauge: Option<(u64, std::time::Instant, f64)>) -> String {
    match gauge {
        Some((avail, when, ema)) => format!(
            "[KERNEL TANK READING — authoritative: ${:.2} available, polled {}s ago, burn ~${:.2}/day. \
Per-model 'last error' rows in the raw payload below persist until that model errors again and can be \
days old; never treat one as current against this line.]\n",
            avail as f64 / 1e6,
            when.elapsed().as_secs(),
            ema * 24.0 / 1e6
        ),
        None => "[No kernel fuel poll yet this run — 'last error' rows in the raw payload below can be \
days stale; verify availableMicros via GET /account before treating any 402 as current.]\n"
            .to_string(),
    }
}

const CEO_TOOL_NAMES: &[&str] = &[
    "hire", "delegate", "delegate_parallel", "dispatch", "team_status", "rate_work", "fire", "list_team",
    "add_routine", "add_review_routine", "remove_routine", "list_routines", "own_routine",
    "update_prompt", "rollback_prompt", "retire_skill", "save_playbook", "finish", "objectives",
    "finish_episode", "message_founder", "ack_founder",
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

/// Hands-on ration for MANAGERS: soft challenge only, no hard stop — a manager
/// legitimately executes, but grinding through volume serially is the disease
/// the crew exists to cure. Founder audit 2026-09-01: in the first day of the
/// org redesign, nine managers issued ONE delegate_parallel between them and
/// the company idled at 1-3 active agents against a 40-seat ceiling — the CEO's
/// ration had simply moved the doom loop one level down. Past this count each
/// budgeted call carries a crew-check reminder; the report is still theirs.
pub(crate) const MANAGER_EXEC_SOFT: u32 = 10;

/// The crew brief injected at the start of every manager task: the live roster
/// with busy/idle state, so delegation needs no list_team call and "I forgot I
/// had a crew" stops being possible. Rows are (name, model, role, is_manager,
/// busy). Pure so the shape is testable.
pub(crate) fn crew_brief(rows: &[(String, String, String, bool, bool)]) -> String {
    let mut lines = String::new();
    for (name, model, role, is_mgr, busy) in rows {
        let state = if *is_mgr { "manager" } else if *busy { "BUSY" } else { "idle" };
        let short: String = role.chars().take(70).collect();
        lines.push_str(&format!("- {name} [{model}] ({state}): {short}\n"));
    }
    format!(
        "[Your crew — the live roster you can delegate to right now]\n{lines}\
You are a MANAGER with delegate_parallel: fan every independent piece of this task out to workers \
running CONCURRENTLY, and hire new specialists freely when the roster lacks one — the seat ceiling \
is far. Your own hands are for judgment, review, and integration; grinding through volume work \
serially in this transcript is the bottleneck the crew exists to remove. A worker marked BUSY is \
mid-task elsewhere — delegate to an idle one or hire."
    )
}

/// The idle-capacity line appended to the CEO's board view every iteration.
/// Computed, not asserted: heartbeat triage kept concluding "every objective
/// is owned" and closing, while the roster measured 51% of 2026-09-01 at
/// zero-or-one active agents against ten open objectives. Owned is not
/// staffed — this line makes the waste a number the model must answer to.
pub(crate) fn idle_capacity_line(open_objectives: usize, busy: usize, roster: usize) -> String {
    let idle = roster.saturating_sub(busy);
    format!(
        "[Idle capacity — computed each iteration] {open_objectives} objectives active, {busy} of {roster} \
workers busy, {idle} idle. An owned lane with no task in flight is idle, not handled. Before \
finish_episode, every idle worker either gets a dispatch that advances an open objective, or your \
closing note states, per lane, why waiting beats working right now. A clock on one lane never idles \
the company — the other lanes keep moving."
    )
}

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

/// The CEO's per-episode ration of hands-on execution — shell, sql, and
/// custom-script runs. Founder doctrine (2026-08-31): the CEO decides and
/// plans who does what; delegation is the point. But finding things out is
/// allowed, so the guard escalates instead of slamming shut: past SOFT every
/// budgeted call still runs but carries an are-you-sure-this-isn't-employee-
/// work challenge; past HARD the tool refuses — that many hands-on calls in
/// one episode is a doom loop, not discovery (350 CEO actions in 2h, 88 of
/// them shell, while its busiest employee logged 90). Every budgeted tool is
/// callable by any employee — sends and kill-exits dispatch like any other
/// work. Plain reads, dispatch, and ratings are unlimited.
///
/// Thresholds calibrated on 2026-08-31's 81 live episodes: 57% used ≤4
/// hands-on calls (discovery — untouched), the 5-12 band was legitimate
/// investigation (challenged, never blocked), and only the 13-15 tail — the
/// exact kill-check/routine-grinding episodes the guard exists for — sat
/// past 12. More than 12 hands-on calls in a 12-step episode is an
/// employee's transcript, not a CEO's.
/// Re-calibrated 2026-09-02 on the previous 24h: 48 episodes sat in the 5-12
/// band and their shell purposes were spot-checks of employee reports, not
/// investigation ("verifying x-mgr's claimed evidence file"). 348 shells in a
/// day, 218 of 269 ratings self-verified.
pub(crate) const CEO_EXEC_SOFT: u32 = 3;
pub(crate) const CEO_EXEC_HARD: u32 = 8;

/// What kind of work a dispatch is, from its leading verbs: "build" advances
/// something (build, ship, write, launch, design…), "check" looks at earlier
/// work (verify, recheck, spot-check, re-run, audit, sweep, checkpoint…),
/// anything else is "other". A heuristic, not a judge — it only has to be
/// right often enough for the budget, the repeat refusal and the board mix
/// to describe the day: on 2026-09-01 it split 384 dispatches 190/188/6 and
/// the check half matched the founder's own read of the log.
pub(crate) fn classify_task(task: &str) -> &'static str {
    let head: String = task.chars().take(160).collect::<String>().to_lowercase();
    const CHECK: &[&str] = &[
        "verify", "re-verify", "recheck", "re-check", "spot-check", "spot check", "re-run", "rerun", "re-read",
        "reconcil", "recon ", "audit", "sweep", "checkpoint", "confirm", "validate", "liveness", "triage",
        "re-validate", "status check", "verify-what-landed", "done check", "read-only",
    ];
    const BUILD: &[&str] = &[
        "build", "ship", "write", "create", "implement", "launch", "deploy", "publish", "design", "redesign",
        "draft", "generate", "execute", "fund", "send", "submit", "post ", "fix", "migrate", "wire",
    ];
    let first_check = CHECK.iter().filter_map(|w| head.find(w)).min();
    let first_build = BUILD.iter().filter_map(|w| head.find(w)).min();
    match (first_build, first_check) {
        (Some(b), Some(c)) => if b <= c { "build" } else { "check" },
        (Some(_), None) => "build",
        (None, Some(_)) => "check",
        (None, None) => "other",
    }
}

/// The shape of a task for repeat detection: objective plus its first six
/// words with punctuation and case folded. "METADATA GATE COIN IMAGE for the
/// PINKPROOF launch" went out four times as four "different" tasks.
pub(crate) fn task_shape(objective: Option<i64>, task: &str) -> String {
    let folded = task
        .chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .take(6)
        .collect::<Vec<_>>()
        .join(" ");
    format!("{}:{folded}", objective.unwrap_or(0))
}

/// The objective a task names in its own words: "obj 34", "objective #9",
/// "obj39". Only the first mention counts; a task about routing one objective's
/// finding to another is the first objective's work.
pub(crate) fn named_objective(task: &str) -> Option<i64> {
    let low = task.to_ascii_lowercase();
    let mut rest = low.as_str();
    while let Some(i) = rest.find("obj") {
        let after = &rest[i + 3..];
        let after = after.strip_prefix("ective").unwrap_or(after);
        let after = after.trim_start_matches(|c: char| matches!(c, ' ' | '#' | '=' | ':'));
        let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = digits.parse::<i64>() {
            if n > 0 {
                return Some(n);
            }
        }
        rest = &rest[i + 3..];
    }
    None
}

/// True when the task names the revenue-idea row it advances: "id65", "id 65",
/// "row 54", "idea 17". A bare "#65" does not count — objectives are written
/// that way, and the whole point is telling the two apart.
pub(crate) fn names_revenue_idea(task: &str) -> bool {
    let low = task.to_ascii_lowercase();
    for key in ["idea", "id", "row"] {
        let mut at = 0;
        while let Some(i) = low[at..].find(key).map(|i| i + at) {
            let starts_word = i == 0 || !low.as_bytes()[i - 1].is_ascii_alphanumeric();
            let after = low[i + key.len()..].trim_start_matches([' ', '#', '=', ':']);
            if starts_word && after.starts_with(|c: char| c.is_ascii_digit()) {
                return true;
            }
            at = i + key.len();
        }
    }
    false
}

/// How many identical shapes in 24h before the next is refused as routine
/// work, and how many consecutive checks on one objective before the next is
/// refused as circling.
pub(crate) const REPEAT_SHAPE_LIMIT: u32 = 3;
pub(crate) const CONSECUTIVE_CHECK_LIMIT: u32 = 3;

/// The refusal a dispatch gets when it is the same shape again or one more
/// check on an objective that has not built anything since. None = allowed.
/// Records the dispatch when allowed so the next call sees it.
pub(crate) fn admit_dispatch(store: &crate::state::Store, agent: &str, objective: Option<i64>, task: &str) -> Option<String> {
    let mut class = classify_task(task);
    // An explore objective's product is a lane, not a longer list. Running the
    // next scan cycle is generation, and the leading-verb classifier scored it
    // build: on 2026-09-02 objective 35 read "7 built / 7 checks" on the board
    // while 16 premise rows sat unmoved and five were past their own review
    // date, so the CONVERT-OR-KILL flag never fired. On an explore objective,
    // only work that names the revenue idea it advances counts as a build.
    if class == "build"
        && objective.is_some_and(|o| o != 0 && store.objective_kind(o) == "explore")
        && !names_revenue_idea(task)
    {
        class = "other";
    }
    let shape = task_shape(objective, task);
    let repeats = store.shape_count_24h(&shape);
    if repeats >= REPEAT_SHAPE_LIMIT {
        return Some(format!(
            "REFUSED: this is the {}th dispatch of the same task shape in 24h ({}). Work that recurs is a ROUTINE — \
             write the check as a script and add_routine it (zero model cost, survives restarts), or if it truly \
             needs judgment each time, say what changed since the last run in the task.",
            repeats + 1,
            shape.split_once(':').map(|(_, t)| t).unwrap_or(&shape)
        ));
    }
    // Upkeep is exempt from the check budget below, so a task that says
    // "obj 34 floor sweep" and is tagged 0 has found the way around it: 12 of
    // the first 43 dispatches after the budget landed (2026-09-02) did exactly
    // that. A task that names an objective is that objective's work.
    if objective.is_none_or(|o| o == 0) {
        if let Some(named) = named_objective(task) {
            return Some(format!(
                "REFUSED: tagged as upkeep (objective 0) but the task names objective #{named}. Dispatch it with objective={named} so the board, owner routing and the check budget see it."
            ));
        }
    }
    // Upkeep (objective 0/None) is exempt from the consecutive-check budget:
    // recon and page-health ARE checks by nature. The repeat-shape refusal
    // still applies to it, which is what turns a recurring check into a routine.
    if class == "check" {
        if let Some(o) = objective.filter(|o| *o != 0) {
            let n = store.consecutive_checks(o);
            if n >= CONSECUTIVE_CHECK_LIMIT {
                return Some(format!(
                    "REFUSED: objective #{o} has had {n} check-class dispatches in a row with nothing built between \
                     them — verifying, re-verifying and sweeping is not advancing it. Dispatch work that BUILDS \
                     something on #{o}, turn the recurring check into a routine, or rate the last report on the \
                     evidence it already carries (txids, row ids, file hashes) and move on."
                ));
            }
        }
    }
    store.record_dispatch(agent, objective, class, &shape);
    None
}

/// The CEO may not run a manager to completion inline. A manager's task is a
/// crew fanned out and reviewed, ten to twenty minutes, and the CEO's episode
/// is the only thing that drains founder messages, alerts and reports: on
/// 2026-09-02 one heartbeat spent 45 minutes in three serial manager delegates
/// while a directive sat undelivered and a KILL alert fired three times
/// unanswered.
pub(crate) fn blocking_manager_run(store: &crate::state::Store, caller: &str, agent: &str) -> Option<String> {
    if caller == "CEO" && store.is_manager(agent) {
        return Some(format!(
            "REFUSED: {agent} is a manager — their run is a whole crew's work and would block this episode for \
             as long as it takes. dispatch({agent}, task, objective) sends them off in the background and their \
             consolidated report opens a new episode when it lands."
        ));
    }
    None
}

/// True when a tool call is the CEO doing work rather than directing or
/// reading: shell, sql, or any custom registry tool (a name that is neither
/// a built-in nor a CEO control tool).
pub(crate) fn ceo_exec_budgeted(tname: &str) -> bool {
    tname == "shell"
        || tname == "sql"
        || (!tools::custom::RESERVED.contains(&tname) && !CEO_TOOL_NAMES.contains(&tname))
}

fn employee_finish_schema() -> Value {
    tool("finish", "Finish the delegated task and report the result to the CEO.", json!({
        "report": {"type": "string"}}), json!(["report"]))
}

fn finish_episode_schema() -> Value {
    tool("finish_episode", "Close this episode with the handoff note for your next self.", json!({
        "note": {"type": "string"}}), json!(["note"]))
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
    /// Every agent currently inside run_employee, by name — dispatch, delegate,
    /// delegate_parallel, and review routines all claim here. `pending` only
    /// covers dispatches, so two managers delegating to the same worker used to
    /// race on its saved history; with heavy fan-out that collision stops being
    /// theoretical, so the claim is enforced where all paths meet.
    pub running: std::sync::Mutex<std::collections::HashSet<String>>,
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
    /// When each model last stalled — a fill cut mid-answer, or one that took
    /// longer than a stall is worth waiting for. Benching used to need a model
    /// to fail through the whole fallback ladder AND not be the floor seat, so
    /// the one model that is both first rung and floor could never be benched:
    /// on 2026-09-02 glm53flash timed out at ~128s twenty-one times in a day
    /// and kept the CEO's seat through every one of them.
    stalls: std::sync::Mutex<std::collections::HashMap<String, Vec<std::time::Instant>>>,
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
        self.chat_fb_capped(agent, model, messages, tools, None).await
    }

    /// A summary is a few thousand tokens whatever the history was; asking for
    /// the model's whole output ceiling only inflates the gateway's reserve.
    async fn chat_fb_capped(&self, agent: &str, model: &str, messages: &[Message], tools: &[Value], cap: Option<u32>) -> Result<(Message, Usage)> {
        let started = std::time::Instant::now();
        self.log_line(agent, "thinking", &format!("{} ({model})", thinking_phrase()));
        match self.llm.chat_capped(&self.ctx.cfg, model, messages, tools, cap).await {
            Ok(r) => {
                // Say so when a model is dragging. Without this a slow provider and a
                // hung one look identical from the outside.
                let secs = started.elapsed().as_secs();
                if secs >= 60 {
                    self.log_line(agent, "slow-model", &format!("{model} took {secs}s to answer"));
                }
                self.ctx.store.record_model_call(model, started.elapsed().as_millis() as u64, true, "", r.1);
                Ok(r)
            }
            Err(e) => {
                self.ctx.store.record_model_call(model, started.elapsed().as_millis() as u64, false, &format!("{e:#}"), Usage::default());
                let why = format!("{e:#}");
                self.log_line(agent, "llm-error", &format!("{model} failed: {why}"));
                // A stall is a fill cut mid-answer (the gateway relays the
                // upstream timeout) or one that burned two minutes to fail.
                // Truncation is the model spending its budget, not the route.
                if truncation(&e).is_none() && is_stall(&why, started.elapsed().as_secs()) {
                    self.record_stall(model, agent);
                }
                // Running out of output budget is normally the one failure another
                // model cannot rescue: the request is unchanged, so every fallback
                // spends its budget the same way. Walking the ladder here only burns
                // minutes and free-tier requests before failing anyway.
                //
                // The exception is a budget we did not choose. When the gateway
                // shrinks the ceiling to what THIS model can produce at its recent
                // speed, a stalling route hands back a ceiling too small to answer
                // inside — 13,824 tokens then 6,400 on 2026-09-02, both spent
                // entirely on reasoning, both of them compaction runs that had to
                // succeed. Another model is quoted its own ceiling, so the ladder
                // is exactly the cure, and the shrunken ceiling is itself evidence
                // this route is degraded.
                if truncation(&e).is_some_and(|t| !t.gateway_capped) {
                    return Err(e);
                }
                if truncation(&e).is_some() {
                    self.record_stall(model, agent);
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
                    match self.llm.chat_capped(&self.ctx.cfg, &alt, messages, tools, cap).await {
                        Ok(r) => {
                            self.ctx.store.record_model_call(&alt, alt_started.elapsed().as_millis() as u64, true, "", r.1);
                            self.log_line(agent, "model-fallback", &format!("{model} failed, answered by {alt}"));
                            return Ok(r);
                        }
                        Err(alt_err) => {
                            self.ctx.store.record_model_call(&alt, alt_started.elapsed().as_millis() as u64, false, &format!("{alt_err:#}"), Usage::default());
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
        match self.chat_fb_capped(name, &self.ctx.cfg.utility_model(), &req, &[], Some(SUMMARY_MAX_TOKENS)).await {
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
        // One body, one task: a second concurrent run would race on the saved
        // history. All paths (dispatch, delegate, delegate_parallel, review
        // routines) claim here; released at the end of the run.
        if !self.running.lock().unwrap().insert(name.to_string()) {
            return format!(
                "ERROR: {name} is already mid-task (a dispatch or another manager is running them). \
                 Delegate to an idle worker or hire a new specialist — the roster is the parallel capacity."
            );
        }
        // "Re-home at next dispatch" was policy text no one executed — agents
        // hired before a seat was denied simply kept running on it. This is that
        // clause, mechanical: the seat moves here, once, on the way into work —
        // inside the one-body lock, so the write cannot race a run of this
        // agent that is still finishing.
        if self.ctx.cfg.seat_denied(&model) {
            let to = self.ctx.cfg.ceo_model.clone();
            self.log_line(name, "re-homed", &format!("seat {model} is denied; moving to {to}"));
            self.ctx.store.save_agent(name, &role, &prompt_name, &to, &hist_json);
            model = to;
        }
        // The home seat stays on record; the run goes to whichever peer the
        // company is actually paying less for. Logged on change only, so the
        // log shows repricings rather than every dispatch.
        let key = format!("peer_seat:{name}");
        if let Some((to, sampled)) = self.cheaper_peer(&model) {
            if self.ctx.store.kv_get(&key).as_deref() != Some(to.as_str()) {
                let why = if sampled { "price sample".to_string() } else { self.peer_reason(&model, &to) };
                self.log_line(name, "peer-seat", &format!("{model} -> {to} ({why})"));
                self.ctx.store.kv_set(&key, &to);
            }
            model = to;
        } else if self.ctx.store.kv_get(&key).is_some_and(|v| !v.is_empty()) {
            self.log_line(name, "peer-seat", &format!("back on {model}"));
            self.ctx.store.kv_set(&key, "");
        }
        // Refuse-don't-drop, same as the history below: a missing prompt row
        // used to hand the employee an EMPTY system prompt silently. Fall back
        // to the base their kind seeds from, and say so where reflection reads.
        let prompt = self.ctx.store.get_prompt(&prompt_name).unwrap_or_else(|| {
            let base_name = if self.ctx.store.is_manager(name) { "manager_base" } else { "employee_base" };
            self.log_line(name, "prompt-error", &format!("prompt '{prompt_name}' is missing; falling back to {base_name}"));
            self.ctx.store.get_prompt(base_name).unwrap_or_default()
        });
        let sys = crate::prompts::employee_system(&prompt.replace("{name}", name).replace("{role}", &role));
        // Refuse-don't-drop: a corrupt saved history used to be silently swapped
        // for a fresh one — the employee lost every prior turn with nothing in
        // the log. Starting fresh is still the only way forward, but it happens
        // loudly where reflection reads.
        let mut history: Vec<Message> = match serde_json::from_str(&hist_json) {
            Ok(h) => h,
            Err(e) => {
                self.log_line(name, "history-error", &format!("saved history failed to parse ({e}); starting fresh"));
                Vec::new()
            }
        };
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
        // Workers have no clock beyond stray log timestamps, so a year-old
        // "September 1" announcement reads as an upcoming event (the fee-change
        // premise incident) — stamp now into every task.
        history.push(Message::text(
            "user",
            format!(
                "It is now {} UTC. Dates before this are the past, and a date without a year belongs to its document's publication date — never assume the current year.\n\nNew task from the CEO:\n{task}",
                chrono::Utc::now().format("%Y-%m-%d %H:%M")
            ),
        ));
        // A manager opens every task seeing its crew — delegation that depends
        // on remembering to call list_team does not happen (measured: one
        // delegate_parallel across nine managers in the redesign's first day).
        if self.ctx.store.is_manager(name) {
            let running = self.running.lock().unwrap().clone();
            let rows: Vec<(String, String, String, bool, bool)> = self
                .ctx
                .store
                .list_agents()
                .into_iter()
                .filter(|(n, _, _)| n != name)
                .map(|(n, r, m)| {
                    let is_mgr = self.ctx.store.is_manager(&n);
                    let busy = running.contains(&n);
                    (n, m, r, is_mgr, busy)
                })
                .collect();
            history.push(Message::text("user", crew_brief(&rows)));
        }

        let mut report = String::from("(employee stopped without a report)");
        let mut fired = false;
        let mut mgr_exec: u32 = 0;

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
            schemas.extend(tools::x::schemas(&self.ctx));
            schemas.extend(tools::gh::schemas(&self.ctx));
            tools::hint_sql_tables(&self.ctx.workspace, &mut schemas);
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
                    // Delegation resets the crew-check: a manager alternating
                    // between directing and spot-checking never sees it.
                    if matches!(call.function.name.as_str(), "delegate" | "delegate_parallel") {
                        mgr_exec = 0;
                    }
                    // Boxed on both sides of the manager/crew cycle so the mutually
                    // recursive futures have a nameable, Send type.
                    let fut: futures::future::BoxFuture<'_, String> =
                        Box::pin(self.ceo_tool(name, &call.function.name, &a));
                    tools::truncate(fut.await)
                } else {
                    let out = tools::execute(&self.ctx, name, &call.function.name, &a).await;
                    // The manager crew-check: soft-only sibling of the CEO exec
                    // ration — one grinding serially through volume hears about
                    // the crew on every budgeted call past the line.
                    if manages && ceo_exec_budgeted(&call.function.name) {
                        mgr_exec += 1;
                        if mgr_exec > MANAGER_EXEC_SOFT {
                            format!(
                                "{out}\n\n[crew check: hands-on call #{mgr_exec} this task. You are a manager with \
                                 delegate_parallel and hire — volume work belongs to workers running in PARALLEL \
                                 while you direct. Fan the independent remainder out now, or note in your report \
                                 why this needed your hands.]"
                            )
                        } else {
                            out
                        }
                    } else {
                        out
                    }
                };
                // A picture rides as the next user turn: tool messages cannot
                // carry image parts, and the model has vision the binary never
                // used until now.
                let picture = tools::image_followup(&self.ctx, &call.function.name, &out);
                history.push(Message::tool_result(&call.id, out));
                if let Some(p) = picture {
                    history.push(p);
                }
            }
            if finished {
                break;
            }
        }
        // The iteration cap used to kill a worker mid-thought — it was never told
        // the end was near, so a worker wedged on a rejecting tool (three stalls
        // in one day on ledger_log_action, 2026-08-30) drained its budget and
        // went silent. Before synthesizing, demand the report directly: one extra
        // call, finish() the only tool on the table.
        if report == "(employee stopped without a report)" && !self.stop.load(Ordering::Relaxed) {
            history.push(Message::text(
                "user",
                "You have hit your iteration limit — this is your final turn. Call finish(report) NOW: \
                 what you completed, exact paths of evidence files you wrote, any transaction ids, what \
                 remains undone, and what blocked you (quote the exact error). Do not start new work.",
            ));
            if let Ok((msg, u)) = self.chat_fb(name, &model, &history, &[employee_finish_schema()]).await {
                self.add_usage(u);
                history.push(msg.clone());
                for call in msg.tool_calls.unwrap_or_default() {
                    if call.function.name == "finish" {
                        report = format!("[filed at the iteration cap]\n{}", s(&args_of(&call), "report"));
                        history.push(Message::tool_result(&call.id, "report delivered"));
                    }
                }
                if report == "(employee stopped without a report)" {
                    if let Some(c) = msg.content.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
                        report = format!("[filed at the iteration cap]\n{c}");
                    }
                }
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
        self.running.lock().unwrap().remove(name);
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
                if self.ctx.cfg.seat_denied(model) {
                    return format!(
                        "ERROR: '{model}' is not a seat — it exists only as the fallback the binary fails through. \
                         Hire onto {} or better.",
                        self.ctx.cfg.ceo_model
                    );
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
                if let Some(why) = blocking_manager_run(&self.ctx.store, caller,&agent) {
                    self.log_line(caller, "dispatch-refused", &why);
                    return why;
                }
                // A manager's delegate has no objective field: the task it fans
                // out names the objective it was dispatched for, and that is
                // its tag. Twelve refusals in four minutes on 2026-09-02 when
                // this path passed None into the upkeep guard.
                if let Some(why) = admit_dispatch(&self.ctx.store, &agent, named_objective(&task), &task) {
                    self.log_line(caller, "dispatch-refused", &why);
                    return why;
                }
                self.run_employee(&agent, &task).await
            }
            "delegate_parallel" => {
                let ts = a["tasks"].as_array().cloned().unwrap_or_default();
                if ts.is_empty() {
                    return "ERROR: tasks must be a non-empty array of {agent, task}".into();
                }
                for t in &ts {
                    if let Some(why) = blocking_manager_run(&self.ctx.store, caller,s(t, "agent")) {
                        self.log_line(caller, "dispatch-refused", &why);
                        return why;
                    }
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
                // Every dispatch names the objective it advances; 0 is company
                // upkeep. Untagged work (173 of 384 on 2026-09-01) was invisible
                // to the board, to owner routing and to every per-objective
                // signal — so the drift it made up most of could not be seen.
                let Some(tagged) = a["objective"].as_i64() else {
                    return "ERROR: dispatch needs `objective`: the board id this task advances, or 0 for company upkeep (bookkeeping, infra, hygiene). Untagged work cannot be routed, counted, or judged.".into();
                };
                let objective = if tagged == 0 { None } else { Some(tagged) };
                if let Some(why) = admit_dispatch(&self.ctx.store, &agent, Some(tagged), &task) {
                    self.log_line(caller, "dispatch-refused", &why);
                    return why;
                }
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
                let owner = s(a, "owner");
                if !owner.is_empty() && self.ctx.store.load_agent(owner).is_none() {
                    return format!("ERROR: no such employee '{owner}' to own this routine. hire them first, or omit owner.");
                }
                let interval = a["interval_secs"].as_i64().unwrap_or(0).max(60);
                self.ctx.store.upsert_routine(name, command, interval, s(a, "purpose"), owner);
                if owner.is_empty() {
                    format!("routine '{name}' scheduled every {interval}s — silent on pass; alerts wake YOU. Assign a domain owner with own_routine so alerts dispatch to them instead.")
                } else {
                    format!("routine '{name}' scheduled every {interval}s — silent on pass; alerts dispatch to {owner}")
                }
            }
            "own_routine" => {
                let (name, owner) = (s(a, "name"), s(a, "owner"));
                if name.is_empty() {
                    return "ERROR: name is required".into();
                }
                if !owner.is_empty() && self.ctx.store.load_agent(owner).is_none() {
                    return format!("ERROR: no such employee '{owner}'. hire them first.");
                }
                if !self.ctx.store.set_routine_owner(name, owner) {
                    return format!("ERROR: no routine named '{name}' — list_routines shows what exists.");
                }
                if owner.is_empty() {
                    format!("routine '{name}' alerts now wake you again")
                } else {
                    format!("routine '{name}' alerts now dispatch to {owner}")
                }
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
                        if let Some(k) = a["kind"].as_str() {
                            if !self.ctx.store.set_objective_kind(id, k) {
                                return format!("ERROR: kind must be profit, growth, infra or explore (got '{k}')");
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
                        if let Some(k) = a["kind"].as_str() {
                            if !["profit", "growth", "infra", "explore"].contains(&k) {
                                return format!("ERROR: kind must be profit, growth, infra or explore (got '{k}')");
                            }
                            ok |= self.ctx.store.set_objective_kind(id, k);
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
            "ack_founder" => {
                let id = a["id"].as_i64().unwrap_or(0);
                if self.ctx.store.ack_message(id) {
                    let note = s(a, "note");
                    self.log_line("CEO", "founder-ack", &format!("directive #{id} done{}", if note.is_empty() { String::new() } else { format!(": {note}") }));
                    format!("directive #{id} acknowledged — it leaves the brief")
                } else {
                    format!("ERROR: no open founder directive #{id} (already acknowledged, or not delivered yet)")
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
        match self.chat_fb_capped("CEO", &self.ctx.cfg.utility_model(), &req, &[], Some(SUMMARY_MAX_TOKENS)).await {
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
        // polluting the rate. The rate sizes a three-day refill, so it has a
        // day of memory: a fixed 0.3 weight per five-minute poll remembered
        // fifteen minutes, and the restart burst of 2026-09-02 walked the
        // target from $25 to $113 in an hour. The last published rate seeds
        // the gauge across restarts so the first poll is not the whole story.
        let burn_per_hour = {
            let mut g = self.seat.gauge.lock().unwrap();
            let now = std::time::Instant::now();
            let ema = match *g {
                Some((prev, t, ema)) if prev > available => {
                    let hrs = (now - t).as_secs_f64() / 3600.0;
                    if hrs > 0.0 {
                        let sample = (prev - available) as f64 / hrs;
                        if ema == 0.0 {
                            sample
                        } else {
                            let a = (hrs / 24.0).min(1.0);
                            ema * (1.0 - a) + sample * a
                        }
                    } else {
                        ema
                    }
                }
                Some((_, _, ema)) => ema,
                None => self
                    .ctx
                    .store
                    .kv_get("fuel_burn_micros_per_hour")
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(0.0),
            };
            *g = Some((available, now, ema));
            ema
        };
        // The kernel's own refill arithmetic, published for the tools and the
        // brief: a top-up is only ever sized to reach this target, so a tank
        // above it has nothing to top up. Stored in kv because the send tools
        // hold a ToolCtx, not the seat gauge.
        let target = {
            let burn_day = burn_per_hour * 24.0;
            ((threshold as f64 + burn_day * 3.0).max(threshold as f64 * 2.5)) as u64
        };
        self.ctx.store.kv_set("fuel_available_micros", &available.to_string());
        self.ctx.store.kv_set("fuel_refill_target_micros", &target.to_string());
        self.ctx.store.kv_set("fuel_burn_micros_per_hour", &(burn_per_hour as u64).to_string());
        if self.ctx.store.kv_get("fuel_deposit_body").is_none() {
            let dep = format!("{}/deposits/solana", provider.base_url.trim_end_matches('/'));
            if let Ok(r) = self.ctx.http.get(&dep).bearer_auth(key).send().await {
                if r.status().is_success() {
                    if let Ok(body) = r.text().await {
                        self.ctx.store.kv_set("fuel_deposit_body", &body);
                    }
                }
            }
        }
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

    /// The peer an agent's dispatch should run on instead of its home seat:
    /// the cheapest peer by realized price when it beats home by more than
    /// `peer_switch_pct`. One dispatch in ten goes to a peer regardless, so a
    /// model nobody is calling keeps a fresh price — a catalog-only rule can
    /// never learn that the loser got cheap again.
    /// The bool says the move is a sample, not a verdict: the first live hour
    /// logged a sample as "luna 206k vs glm 78k" and read as the switch
    /// picking the dearer seat.
    fn cheaper_peer(&self, home: &str) -> Option<(String, bool)> {
        let peers: Vec<String> = self
            .ctx
            .cfg
            .peers_of(home)
            .into_iter()
            .filter(|p| {
                self.ctx
                    .store
                    .success_rate(p, PEER_PRICE_HOURS)
                    .is_none_or(|(ok, n)| ok * 100 >= n * PEER_MIN_OK_PCT)
            })
            .collect();
        if peers.is_empty() {
            return None;
        }
        if chrono::Utc::now().timestamp() % 10 == 0 {
            return peers.first().cloned().map(|p| (p, true));
        }
        let mine = self.ctx.store.realized_price(home, PEER_PRICE_HOURS)?;
        let best = peers
            .iter()
            .filter_map(|p| self.ctx.store.realized_price(p, PEER_PRICE_HOURS).map(|c| (c, p.clone())))
            .min()?;
        (best.0 * 100 < mine * (100 - self.ctx.cfg.peer_switch_pct)).then_some((best.1, false))
    }

    fn peer_reason(&self, home: &str, to: &str) -> String {
        match (self.ctx.store.realized_price(home, PEER_PRICE_HOURS), self.ctx.store.realized_price(to, PEER_PRICE_HOURS)) {
            (Some(h), Some(t)) => format!("realized {t} vs {h} micro$/1M tokens over {PEER_PRICE_HOURS}h"),
            _ => "price sample".to_string(),
        }
    }

    /// The seat picked for the episode in flight (bookkeeping paths only).
    fn current_ceo_model(&self) -> String {
        let cur = self.seat.current.lock().unwrap();
        if cur.is_empty() { self.ctx.cfg.ceo_model.clone() } else { cur.clone() }
    }

    /// Record a stalled call and bench the model once they cluster.
    ///
    /// Benching the first rung is safe because the picker falls back to
    /// `cfg.ceo_model` when every rung is benched — the company can always
    /// still think, it just stops waiting on a route that is cutting answers
    /// off. Recorded for any agent's call: a bad route is bad for everyone,
    /// and the CEO alone would take an hour to gather the evidence.
    fn record_stall(&self, model: &str, agent: &str) {
        let now = std::time::Instant::now();
        let hits = {
            let mut s = self.seat.stalls.lock().unwrap();
            let e = s.entry(model.to_string()).or_default();
            stall_strike(e, now)
        };
        if hits < STALL_STRIKES {
            return;
        }
        // Only bench while something else can take the seat; benching the last
        // model standing would buy nothing and cost the log line.
        let has_alternative = self.ctx.cfg.ceo_models.iter().any(|m| {
            m != model && !self.seat.cooldown.lock().unwrap().get(m).is_some_and(|u| *u > now)
        });
        if !has_alternative {
            return;
        }
        self.seat.stalls.lock().unwrap().remove(model);
        self.bench_seat(model);
        self.log_line(
            agent,
            "seat-benched",
            &format!("{model} stalled {hits} times in 10m — benched 15m, the ladder drops a rung"),
        );
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
    /// heartbeat_secs doubled per consecutive quiet heartbeat that dispatched
    /// nothing, capped at heartbeat_backoff_max_secs.
    pub(crate) fn heartbeat_interval(&self) -> u64 {
        let base = self.ctx.cfg.heartbeat_secs;
        let max = self.ctx.cfg.heartbeat_backoff_max_secs;
        if max == 0 {
            return base;
        }
        let level = self.ctx.store.kv_get("heartbeat_backoff_level").and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
        backoff_interval(base, max, level)
    }

    fn heartbeat_due(&self) -> bool {
        let now = chrono::Utc::now();
        let last = self
            .ctx
            .store
            .kv_get("last_heartbeat")
            .and_then(|v| chrono::DateTime::parse_from_rfc3339(&v).ok())
            .map(|t| t.with_timezone(&chrono::Utc));
        match last {
            Some(t) if (now - t).num_seconds() as u64 >= self.heartbeat_interval() => {
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
            // Standing policy rides the config, not the CEO's memory: a restart
            // forgot the nudged hiring policy within the hour.
            let policy = self
                .ctx
                .cfg
                .model_policy
                .as_deref()
                .map(|p| format!("\n\nMODEL POLICY from your founder (standing — applies to every seat and hire):\n{p}"))
                .unwrap_or_default();
            let directives = open_directives_text(&self.ctx.store.open_directives());
            let fuel = fuel_brief_line(&self.ctx.store);
            let ideas = overdue_ideas_line(&self.ctx.workspace);
            history.push(Message::text(
                "user",
                format!(
                    "[Company brief — composed fresh each episode; durable truth lives on the objective board, in memories and in skills]\n\
It is now {now} UTC. Anything dated before this already happened. A dated announcement is history, not a catalyst — and a date WITHOUT a year never resolves to the current calendar: it resolves to the document's own publication date (commit date, Last-Modified, weekday arithmetic). Check that before treating any date as upcoming.\n\n\
BASE DIRECTIVE from your founder:\n{directive}\n\nTEAM:\n{roster}{policy}{directives}{fuel}{ideas}\n\nRECENT ACTIVITY (public log tail):\n{recent}",
                    now = chrono::Utc::now().format("%Y-%m-%d %H:%M")
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
            // The CEO's own hands-on execution this episode — shell, sql, and
            // custom-script runs. Bounded because instruction alone did not
            // hold: on 2026-08-31 the CEO logged 350 actions in 2h (88 shells,
            // hand-running kill-checks, re-running green routines) while its
            // busiest employee logged 90. Directing and reading stay unlimited;
            // doing is budgeted, and past the cap the tool refuses with a
            // dispatch redirect.
            let mut exec_spent: u32 = 0;
            let mut obs_streak: u32 = 0;
            // Whether this episode put anyone to work; a quiet heartbeat that
            // did not backs the next heartbeat off.
            let mut dispatched: bool = false;
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
            // An open founder directive is work: the first two-step heartbeat
            // after the cap shipped spent both steps on directive #151 and hit
            // the cut-off mid-task.
            // And an idle team is not quiet either: the heartbeat is then the
            // only thing that can start work. After the 02:56Z restart on
            // 2026-09-02 killed five in-flight dispatches, three capped
            // heartbeats in a row read the board, ran out of steps, dispatched
            // nothing, and backed off to twenty minutes with 34 idle workers.
            let quiet_heartbeat = heartbeat
                && !self.ctx.store.has_pending_input()
                && self.ctx.store.open_directives().is_empty()
                && {
                    let p = self.pending.lock().await;
                    !p.is_empty() && !p.iter().any(|t| t.handle.is_finished())
                };
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
            // A quiet heartbeat is a status sweep: one look, one action or
            // close. The full cap let it spot-check its way to the cut-off —
            // 786 heartbeat steps on 2026-09-01, 88 of 151 dispatching nothing.
            // The moment work drains in it is no longer quiet and gets the
            // full budget.
            let cap = if quiet_heartbeat && !work_arrived {
                self.ctx.cfg.quiet_heartbeat_max_steps.min(self.ctx.cfg.episode_max_steps)
            } else {
                self.ctx.cfg.episode_max_steps
            };
            if steps > cap {
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
            schemas.extend(tools::x::schemas(&self.ctx));
            schemas.extend(tools::gh::schemas(&self.ctx));
            tools::hint_sql_tables(&self.ctx.workspace, &mut schemas);
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
            for (msg_id, m) in self.ctx.store.drain_messages() {
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
                let standing = if m.starts_with("[via Telegram]") {
                    String::new()
                } else {
                    format!(" — directive #{msg_id}: it stays in your brief every episode until ack_founder({msg_id}) once it is actually done")
                };
                history.push(Message::text("user", format!("[Message from your founder — act on this now{standing}]\n{m}")));
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

            // The reflection payload opens every heartbeat episode that has
            // room to act on it. A quiet heartbeat has two steps: sending it
            // the ratings, skill, model and portfolio tables (rebuilt 151
            // times a day) bought nothing but tokens.
            if heartbeat && steps == 1 && !quiet_heartbeat {
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
                        "\n\nCREDIT BURN — prepaid balance and recent usage (raw):\n{}{snap}\n\
You are currently running on {ceo_model} — your seat is picked automatically by the binary: the best \
approved model whose live marketplace price fits the configured ceilings, with the default as floor. \
Credits are finite: project the runway at the current pace. If the runway is short, that is a \
treasury decision: top up, or cut the burn.",
                        fuel_anchor(*self.seat.gauge.lock().unwrap())
                    ),
                    None => String::new(),
                };
                // Once a week the reflection widens into a portfolio review:
                // each category of objective judged by its own yardstick, so a
                // growth lane is never killed for earning nothing and "it's
                // marketing" never excuses unlimited spend. Weekly because the
                // per-bet kill-checks already run daily; this is the step-back.
                let review_due = self
                    .ctx
                    .store
                    .kv_get("portfolio_review_at")
                    .and_then(|t| chrono::DateTime::parse_from_rfc3339(&t).ok())
                    .map(|t| (chrono::Utc::now() - t.with_timezone(&chrono::Utc)).num_days() >= 7)
                    .unwrap_or(true);
                let portfolio_review = if review_due {
                    let since = self
                        .ctx
                        .store
                        .kv_get("portfolio_review_at")
                        .unwrap_or_else(|| (chrono::Utc::now() - chrono::Duration::days(7)).to_rfc3339());
                    let body = self.ctx.store.portfolio_review_text(&since);
                    if body.is_empty() {
                        String::new()
                    } else {
                        self.ctx.store.kv_set("portfolio_review_at", &chrono::Utc::now().to_rfc3339());
                        format!(
                            "\n\nWEEKLY PORTFOLIO REVIEW — step back from the lanes and judge the allocation itself. \
For profit lanes, pull the actual numbers from the books (revenue booked vs spend) before judging — attention share \
below is measured, revenue is yours to join. Record a one-line verdict per lane in its board note, and act on the \
verdicts: kill measured losers, scale measured winners, re-cap growth envelopes, and reclassify anything mislabeled.\n{body}"
                        )
                    }
                } else {
                    String::new()
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
Save one-off lessons with save_playbook. Then continue the mission.\n\n{log}{stats_block}{skill_block}{capacity_block}{portfolio_block}{portfolio_review}{health_block}{catalog}{model_block}{untried_block}{burn_block}\n\n{toks}"
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
                // Roster utilization rides with the board, computed the same
                // way the crew brief is for managers: visibility that does not
                // depend on the model remembering to ask.
                {
                    let running = self.running.lock().unwrap().clone();
                    let roster = self.ctx.store.list_agents();
                    let busy = roster.iter().filter(|(n, _, _)| running.contains(n)).count();
                    let open = self.ctx.store.active_objective_count();
                    req.push(Message::text("user", idle_capacity_line(open, busy, roster.len())));
                }
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
                    if matches!(tname.as_str(), "dispatch" | "delegate" | "delegate_parallel" | "hire") {
                        dispatched = true;
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
                    } else if ceo_exec_budgeted(&tname) {
                        exec_spent += 1;
                        if exec_spent > CEO_EXEC_HARD {
                            self.log_line("CEO", "exec-budget", &format!("{tname} refused — {CEO_EXEC_HARD} hands-on calls this episode is a doom loop"));
                            format!(
                                "REFUSED: this is hands-on call #{exec_spent} this episode — past discovery, into a doom loop. \
                                 You are the CEO: every one of these tools is callable by an employee. Dispatch the work with \
                                 clear instructions and rate the result, or close the episode."
                            )
                        } else if exec_spent > CEO_EXEC_SOFT {
                            let out = tools::execute(&self.ctx, "CEO", &tname, &a).await;
                            format!(
                                "{out}\n\n[exec check: hands-on call #{exec_spent} this episode — are you sure this is CEO work, \
                                 or should it be delegated? An employee can run this same tool; at {CEO_EXEC_HARD} the tool refuses.]"
                            )
                        } else {
                            tools::execute(&self.ctx, "CEO", &tname, &a).await
                        }
                    } else {
                        tools::execute(&self.ctx, "CEO", &tname, &a).await
                    };
                    let picture = tools::image_followup(&self.ctx, &tname, &out);
                    history.push(Message::tool_result(&call.id, out));
                    if let Some(p) = picture {
                        history.push(p);
                    }
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

            // The cut-offs used to end the episode without a word — the step cap
            // and the quiescence break both just left 'turns, and 40% of
            // episodes closed synthesized (165 of 409 on 2026-08-31). Same
            // disease the employee loop had at its iteration cap, same cure:
            // before synthesizing, demand the handoff directly — one extra
            // call, finish_episode the only tool on the table.
            if episode_note.is_none() && !self.stop.load(Ordering::Relaxed) {
                history.push(Message::text(
                    "user",
                    "This episode is over — this is your final turn. Call finish_episode(note) NOW: \
                     what changed, what is in flight and with whom, and what the next episode must \
                     do or know. Do not start new work.",
                ));
                if let Ok((msg, u)) = self.chat_fb("CEO", &ceo_model, &history, &[finish_episode_schema()]).await {
                    self.add_usage(u);
                    history.push(msg.clone());
                    for call in msg.tool_calls.unwrap_or_default() {
                        if call.function.name == "finish_episode" {
                            episode_note =
                                Some(format!("[filed at the cut-off]\n{}", s(&args_of(&call), "note").chars().take(1500).collect::<String>()));
                            history.push(Message::tool_result(&call.id, "episode closed"));
                        }
                    }
                    if episode_note.is_none() {
                        if let Some(c) = msg.content.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
                            episode_note = Some(format!("[filed at the cut-off]\n{}", c.chars().take(1500).collect::<String>()));
                        }
                    }
                }
            }

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
            // Heartbeat backoff: a quiet heartbeat that put no one to work
            // doubles the wait to the next one, up to the ceiling; anything
            // that drains in resets it. Silence gets cheaper, events do not.
            if self.ctx.cfg.heartbeat_backoff_max_secs > 0 {
                if work_arrived || dispatched || !heartbeat {
                    self.ctx.store.kv_set("heartbeat_backoff_level", "0");
                } else if quiet_heartbeat {
                    let level = self.ctx.store.kv_get("heartbeat_backoff_level").and_then(|v| v.parse::<u32>().ok()).unwrap_or(0) + 1;
                    self.ctx.store.kv_set("heartbeat_backoff_level", &level.to_string());
                    self.log_line("CEO", "heartbeat-backoff", &format!("quiet heartbeat dispatched nothing — next in {}s", self.heartbeat_interval()));
                }
            }
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
