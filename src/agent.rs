use crate::llm::{Client, Message, Usage};
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
        tool("hire", "Hire a new employee agent. Their role prompt persists and evolves across runs.", json!({
            "name": {"type": "string", "description": "Short unique name, e.g. 'researcher-1'"},
            "role": {"type": "string", "description": "What they do and how, in a paragraph"},
            "model": {"type": "string", "description": "provider/model to run them on"}}),
            json!(["name", "role", "model"])),
        tool("delegate", "Give an existing employee a task. Runs them to completion and returns their report.", json!({
            "agent": {"type": "string"}, "task": {"type": "string"}}),
            json!(["agent", "task"])),
        tool("delegate_parallel", "Give several employees independent tasks that run CONCURRENTLY. Returns all reports. Use this whenever tasks don't depend on each other — it keeps the whole team busy.", json!({
            "tasks": {"type": "array", "items": {"type": "object", "properties": {
                "agent": {"type": "string"}, "task": {"type": "string"}},
                "required": ["agent", "task"]}}}),
            json!(["tasks"])),
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

const CEO_TOOL_NAMES: &[&str] = &[
    "hire", "delegate", "delegate_parallel", "rate_work", "fire", "list_team",
    "update_prompt", "rollback_prompt", "save_playbook", "finish",
];

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

pub struct Orchestrator {
    pub ctx: ToolCtx,
    pub llm: Client,
    pub stop: Arc<AtomicBool>,
    pub tokens: Tokens,
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
                Ok(r)
            }
            Err(e) => {
                self.log_line(agent, "llm-error", &format!("{model} failed: {e:#}"));
                for alt in self.ctx.cfg.free_model_ids() {
                    if alt == model {
                        continue;
                    }
                    self.log_line(agent, "thinking", &format!("{} ({alt})", thinking_phrase()));
                    match self.llm.chat(&self.ctx.cfg, &alt, messages, tools).await {
                        Ok(r) => {
                            self.log_line(agent, "model-fallback", &format!("{model} failed, answered by {alt}"));
                            return Ok(r);
                        }
                        Err(alt_err) => {
                            self.log_line(agent, "llm-error", &format!("{alt} failed too: {alt_err:#}"));
                        }
                    }
                }
                Err(e)
            }
        }
    }

    /// Compact once the history passes this many characters. Every character here is
    /// re-sent and re-billed on EVERY iteration: the CEO runs on a marketplace router
    /// that picks the cheapest pod per request, so there is no prompt cache to hit and
    /// nothing is amortised. It is also paid again in latency on the first call after
    /// a restart, which is what made resumes take minutes rather than seconds.
    const COMPACT_AT: usize = 100_000;
    /// Characters of the most recent turns kept verbatim. Recency is what the agent
    /// needs to continue the exact thing it was doing; older detail goes to the brief.
    const KEEP_RECENT: usize = 40_000;

    fn history_chars(history: &[Message]) -> usize {
        history.iter().map(|m| m.content.as_deref().map_or(0, |c| c.len())).sum()
    }

    async fn maybe_compact(&self, name: &str, history: &mut Vec<Message>) {
        if Self::history_chars(history) < Self::COMPACT_AT || history.len() < 20 {
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

    fn persist_ceo(&self, history: &[Message]) {
        let h = serde_json::to_string(history).unwrap_or_else(|_| "[]".into());
        self.ctx.store.save_agent("CEO", "CEO", "CEO", &self.ctx.cfg.ceo_model, &h);
    }

    /// Run one employee's loop to completion on a task; returns their report.
    /// Takes &self so several employees can run concurrently (delegate_parallel).
    async fn run_employee(&self, name: &str, task: &str) -> String {
        let Some((role, prompt_name, model, hist_json)) = self.ctx.store.load_agent(name) else {
            return format!("ERROR: no such employee '{name}'. hire them first or check list_team.");
        };
        let sys = self
            .ctx
            .store
            .get_prompt(&prompt_name)
            .unwrap_or_default()
            .replace("{name}", name)
            .replace("{role}", &role)
            + crate::prompts::SECURITY;
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

        for _ in 0..self.ctx.cfg.employee_max_iters {
            if self.stop.load(Ordering::Relaxed) {
                report = "(interrupted by shutdown)".into();
                break;
            }
            self.maybe_compact(name, &mut history).await;
            // Rebuilt every iteration so custom tools created anywhere show up immediately.
            let mut schemas = tools::work_schemas();
            schemas.extend(tools::custom::management_schemas());
            schemas.extend(tools::custom::registry_schemas(&self.ctx));
            schemas.extend(tools::skills::schemas());
            schemas.extend(tools::credits::schemas(&self.ctx));
            schemas.push(employee_finish_schema());
            let (msg, u) = match self.chat_fb(name, &model, &history, &schemas).await {
                Ok(r) => r,
                Err(e) => {
                    report = format!("ERROR: employee '{name}' model call failed: {e:#}");
                    break;
                }
            };
            self.add_usage(u);
            history.push(msg.clone());
            let calls = msg.tool_calls.unwrap_or_default();
            if calls.is_empty() {
                // Model answered in prose; treat it as the report.
                report = msg.content.unwrap_or_default();
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
                let out = tools::execute(&self.ctx, name, &call.function.name, &a).await;
                history.push(Message::tool_result(&call.id, out));
            }
            if finished {
                break;
            }
        }
        let h = serde_json::to_string(&history).unwrap_or_else(|_| "[]".into());
        self.ctx.store.save_agent(name, &role, &prompt_name, &model, &h);
        self.log_line(name, "report", &report);
        report
    }

    /// Execute a CEO control tool.
    async fn ceo_tool(&self, name: &str, a: &Value) -> String {
        match name {
            "hire" => {
                let (n, role, model) = (s(a, "name"), s(a, "role"), s(a, "model"));
                if self.ctx.cfg.resolve(model).is_err() {
                    return format!("ERROR: model '{model}' is not available. Pick from the catalog in your instructions.");
                }
                let prompt_name = format!("agent:{n}");
                if self.ctx.store.get_prompt(&prompt_name).is_none() {
                    let base = self.ctx.store.get_prompt("employee_base").unwrap_or_default();
                    self.ctx.store.seed_prompt(&prompt_name, &base);
                }
                self.ctx.store.save_agent(n, role, &prompt_name, model, "[]");
                format!("hired {n} ({role}) on {model}")
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
                if self.ctx.store.fire_agent(s(a, "name")) { "fired".into() } else { "no such employee".into() }
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
            _ => format!("unknown tool {name}"),
        }
    }

    /// The unbounded CEO loop. Runs until the stop flag is set (Ctrl+C).
    pub async fn run_ceo(&self, directive: &str, fresh: bool) -> Result<()> {
        let sys = self.ctx.store.get_prompt("CEO").unwrap_or_default() + crate::prompts::SECURITY;
        let mut history: Vec<Message> = if fresh {
            vec![
                Message::text("system", sys),
                Message::text("user", format!("BASE DIRECTIVE from your founder:\n{directive}\n\nBegin. Work autonomously and continuously.")),
            ]
        } else {
            let h = self.ctx.store.load_agent("CEO").map(|(_, _, _, h)| h).unwrap_or_else(|| "[]".into());
            let mut hist: Vec<Message> = serde_json::from_str(&h).unwrap_or_default();
            if hist.is_empty() {
                hist.push(Message::text("system", sys));
                hist.push(Message::text("user", format!("BASE DIRECTIVE from your founder:\n{directive}\n\nBegin.")));
            } else {
                hist[0] = Message::text("system", sys); // pick up evolved prompt
                hist.push(Message::text("user", "You were restarted. Review where you left off (use recall / list_team / read the workspace) and continue."));
            }
            hist
        };

        let mut iter: u64 = self.ctx.store.kv_get("iteration").and_then(|v| v.parse().ok()).unwrap_or(0);

        loop {
            if self.stop.load(Ordering::Relaxed) {
                break;
            }
            iter += 1;
            self.ctx.store.kv_set("iteration", &iter.to_string());
            self.maybe_compact("CEO", &mut history).await;
            // Rebuilt every iteration so newly created custom tools become callable at once.
            let mut schemas = tools::work_schemas();
            schemas.extend(tools::custom::management_schemas());
            schemas.extend(tools::custom::registry_schemas(&self.ctx));
            schemas.extend(tools::skills::schemas());
            schemas.extend(tools::credits::schemas(&self.ctx));
            schemas.extend(ceo_schemas());

            // Founder messages sent via `khan tell` land as top-priority instructions.
            for m in self.ctx.store.drain_messages() {
                self.log_line("CEO", "founder-message", &m);
                history.push(Message::text("user", format!("[Message from your founder — act on this now]\n{m}")));
            }

            // Reflection cadence: ask the CEO to evolve itself.
            if iter % self.ctx.cfg.reflect_every == 0 {
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
Save one-off lessons with save_playbook. Then continue the mission.\n\n{log}{stats_block}{health_block}{catalog}\n\n{toks}"
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

            let (msg, u) = match self.chat_fb("CEO", &self.ctx.cfg.ceo_model, &req, &schemas).await {
                Ok(r) => r,
                Err(e) => {
                    self.log_line("CEO", "llm-error", &format!("{e:#}"));
                    tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                    continue;
                }
            };
            self.add_usage(u);
            if let Some(c) = &msg.content {
                if !c.trim().is_empty() {
                    self.log_line("CEO", "says", c);
                }
            }
            history.push(msg.clone());

            let calls = msg.tool_calls.unwrap_or_default();
            if calls.is_empty() {
                history.push(Message::text(
                    "user",
                    "Do not stop. Take the next concrete action with a tool call (or finish(report) if you hit a milestone).",
                ));
            } else {
                for call in calls {
                    let a = args_of(&call);
                    let tname = call.function.name.clone();
                    self.log_line("CEO", &tname, &call.function.arguments);
                    let out = if CEO_TOOL_NAMES.contains(&tname.as_str()) {
                        tools::truncate(self.ceo_tool(&tname, &a).await)
                    } else {
                        tools::execute(&self.ctx, "CEO", &tname, &a).await
                    };
                    history.push(Message::tool_result(&call.id, out));
                }
            }

            self.persist_ceo(&history);
            if iter % 5 == 0 {
                println!(
                    "\x1b[2m-- iter {iter} | tokens: {} in / {} out --\x1b[0m",
                    self.tokens.prompt.load(Ordering::Relaxed),
                    self.tokens.completion.load(Ordering::Relaxed)
                );
            }
        }
        // Save the real history FIRST, so nothing is lost if the platform kills us
        // during the summarization below.
        self.persist_ceo(&history);
        // Then shrink what the next boot has to load. A resume re-sends the whole
        // history in one uncached call before it can act, which is the difference
        // between coming back in seconds and coming back in minutes. Bounded, because
        // a shutdown that overruns the platform's grace period gets SIGKILLed.
        if Self::history_chars(&history) > Self::KEEP_RECENT {
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(60),
                self.compact("CEO", &mut history),
            )
            .await;
            self.persist_ceo(&history);
        }
        self.ctx.store.log("khan", "shutdown", "state saved — resumes on next start");
        println!("\nState saved. Resume with: khan resume");
        Ok(())
    }
}
