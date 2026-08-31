use anyhow::Result;
use rusqlite::{params, Connection};
use std::sync::Mutex;
use tokio::sync::broadcast;

fn log_row_json(id: i64, ts: &str, agent: &str, event: &str, detail: &str) -> String {
    serde_json::json!({"id": id, "ts": ts, "agent": agent, "event": event, "detail": detail}).to_string()
}

fn is_tok(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

fn looks_secret(t: &str) -> bool {
    let n = t.len();
    if t.starts_with("sk-") && n >= 20 {
        return true;
    }
    if t.starts_with("bu0y_") && n >= 16 {
        return true;
    }
    // 64+ hex chars: generic private key material.
    if n >= 64 && t.chars().all(|c| c.is_ascii_hexdigit()) {
        return true;
    }
    // 64+ base58: a Solana secret key is 87-88 chars. Public addresses are only
    // 32-44, and the company publishes those on purpose, so the threshold sits
    // above them deliberately.
    if n >= 64 && t.chars().all(|c| c.is_ascii_alphanumeric() && !"0OIl".contains(c)) {
        return true;
    }
    false
}

/// If a JSON byte array starts at `i`, return (index just past `]`, element count).
fn byte_array(b: &[char], i: usize) -> Option<(usize, usize)> {
    let ws = |c: char| matches!(c, ' ' | '\n' | '\r' | '\t');
    let mut j = i + 1;
    let mut n = 0usize;
    loop {
        while j < b.len() && ws(b[j]) {
            j += 1;
        }
        let start = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j == start || j - start > 3 {
            return None;
        }
        n += 1;
        while j < b.len() && ws(b[j]) {
            j += 1;
        }
        match b.get(j)? {
            ',' => j += 1,
            ']' => return Some((j + 1, n)),
            _ => return None,
        }
    }
}

/// Exact values to strike from log text, paired with what to show instead.
///
/// The shape matcher below can only catch things that *look* like key material.
/// A private endpoint does not: it is an ordinary URL, and the only thing that
/// distinguishes it from a public one is that we know its value. So we register
/// the value.
static LITERALS: std::sync::OnceLock<std::sync::RwLock<Vec<(String, String)>>> =
    std::sync::OnceLock::new();

fn literals() -> &'static std::sync::RwLock<Vec<(String, String)>> {
    LITERALS.get_or_init(|| std::sync::RwLock::new(Vec::new()))
}

/// Register a value that must never appear in the public log, shown as
/// `[REDACTED-<label>]` wherever it occurs.
///
/// Registers the whole value and, for a URL, the long opaque tokens inside it —
/// so printing just the key out of an endpoint is caught as well as printing the
/// endpoint. Tokens containing a dot are skipped: those are hostnames, and
/// blanking every mention of a host makes the log unreadable without hiding
/// anything that is actually secret.
///
/// Short values are ignored outright. A 4-character "secret" is a substring of
/// ordinary English, and redacting it would shred the log while protecting
/// nothing.
pub fn redact_value(value: &str, label: &str) {
    let v = value.trim();
    if v.len() < 12 {
        return;
    }
    let mut lits = literals().write().unwrap();
    let mut add = |t: &str| {
        if t.len() >= 12 && !lits.iter().any(|(x, _)| x == t) {
            lits.push((t.to_string(), format!("[REDACTED-{label}]")));
        }
    };
    add(v);
    for tok in v.split(['/', '?', '&', '=', '@', ':']) {
        if tok.len() >= 20 && !tok.contains('.') {
            add(tok);
        }
    }
    // Longest first, so a full URL is replaced as a unit rather than being left
    // as a shell of punctuation around an already-redacted inner token.
    lits.sort_by_key(|(v, _)| std::cmp::Reverse(v.len()));
}

/// Strip secret-shaped strings out of activity-log text.
///
/// The activity log is served to anyone on the internet by the web viewer, so
/// this is the one place a leaked key can be stopped *in code* rather than by
/// asking the model nicely (prompts.rs SECURITY rule 3). It runs before the DB
/// insert, so the replayed history is scrubbed too — not just the live stream.
///
/// Covers: Solana secret keys (base58 and the 64-byte JSON keypair form),
/// 64+ char hex key material, and sk-/bu0y_ API keys. It does NOT detect
/// mnemonic seed phrases — those are ordinary words and cannot be matched
/// without a wordlist.
pub fn redact(s: &str) -> String {
    // Exact known values first. Shape matching cannot help with something like a
    // private RPC endpoint - it is just a URL - so those are struck by literal.
    let mut cow = std::borrow::Cow::Borrowed(s);
    for (v, label) in literals().read().unwrap().iter() {
        if cow.contains(v.as_str()) {
            cow = std::borrow::Cow::Owned(cow.replace(v.as_str(), label));
        }
    }
    let b: Vec<char> = cow.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == '[' {
            if let Some((end, n)) = byte_array(&b, i) {
                if n >= 32 {
                    out.push_str("[REDACTED-KEYPAIR]");
                    i = end;
                    continue;
                }
            }
        }
        if is_tok(b[i]) && (i == 0 || !is_tok(b[i - 1])) {
            let mut j = i;
            while j < b.len() && is_tok(b[j]) {
                j += 1;
            }
            if looks_secret(&b[i..j].iter().collect::<String>()) {
                // Keep a short prefix. A Solana transaction signature has exactly the
                // same shape as a secret key (both 64 bytes base58), so this cannot
                // tell them apart — and signatures are the on-chain proof of work the
                // public log exists to show. Six characters is enough to match one on
                // an explorer, and far too little to weaken a key.
                out.extend(b[i..j].iter().take(6));
                out.push_str("...[REDACTED]");
                i = j;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// Internal persistent state (khan.db): prompts, agents, run log, memories, kv.
pub struct Store {
    pub conn: Mutex<Connection>,
    /// Live feed of run_log rows as JSON, consumed by the web log viewer.
    log_tx: broadcast::Sender<String>,
}

impl Store {
    pub fn open(path: &str) -> Result<Store> {
        let conn = Connection::open(path)?;
        // A second process (khan tell) may write while the main loop runs.
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS kv (k TEXT PRIMARY KEY, v TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS prompts (
                name TEXT NOT NULL, version INTEGER NOT NULL, content TEXT NOT NULL,
                reason TEXT, created_at TEXT NOT NULL, PRIMARY KEY (name, version));
             CREATE TABLE IF NOT EXISTS agents (
                name TEXT PRIMARY KEY, role TEXT NOT NULL, prompt_name TEXT NOT NULL,
                model TEXT NOT NULL, history TEXT NOT NULL DEFAULT '[]', active INTEGER NOT NULL DEFAULT 1);
             CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT, msg TEXT NOT NULL,
                created_at TEXT NOT NULL, delivered INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE IF NOT EXISTS skill_loads (
                id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT NOT NULL,
                agent TEXT NOT NULL, skill TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS telegram_chat (
                id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT NOT NULL,
                role TEXT NOT NULL, text TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS ratings (
                id INTEGER PRIMARY KEY AUTOINCREMENT, agent TEXT NOT NULL, score INTEGER NOT NULL,
                note TEXT, prompt_version INTEGER, created_at TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS run_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT NOT NULL,
                agent TEXT NOT NULL, event TEXT NOT NULL, detail TEXT);
             CREATE TABLE IF NOT EXISTS tool_calls (
                id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT NOT NULL,
                tool TEXT NOT NULL, ok INTEGER NOT NULL, err TEXT);
             CREATE TABLE IF NOT EXISTS model_calls (
                id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT NOT NULL,
                model TEXT NOT NULL, ms INTEGER NOT NULL, ok INTEGER NOT NULL, err TEXT);
             CREATE TABLE IF NOT EXISTS routines (
                name TEXT PRIMARY KEY, command TEXT NOT NULL, interval_secs INTEGER NOT NULL,
                purpose TEXT, enabled INTEGER NOT NULL DEFAULT 1,
                last_run INTEGER NOT NULL DEFAULT 0, last_status TEXT);
             CREATE TABLE IF NOT EXISTS routine_alerts (
                id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT NOT NULL,
                name TEXT NOT NULL, detail TEXT NOT NULL, delivered INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE IF NOT EXISTS episodes (
                id INTEGER PRIMARY KEY AUTOINCREMENT, started_at TEXT NOT NULL,
                ended_at TEXT NOT NULL, event_kind TEXT NOT NULL,
                note TEXT NOT NULL, steps INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS objectives (
                id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL,
                rank INTEGER NOT NULL DEFAULT 100, status TEXT NOT NULL DEFAULT 'active',
                plan TEXT NOT NULL DEFAULT '', note TEXT NOT NULL DEFAULT '',
                blocked_by TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS tool_defs (
                name TEXT NOT NULL, version INTEGER NOT NULL, description TEXT NOT NULL,
                params TEXT NOT NULL, lang TEXT NOT NULL, script TEXT NOT NULL,
                reason TEXT, created_at TEXT NOT NULL, PRIMARY KEY (name, version));
             CREATE TABLE IF NOT EXISTS skill_defs (
                name TEXT NOT NULL, version INTEGER NOT NULL, description TEXT NOT NULL,
                content TEXT NOT NULL, reason TEXT, created_at TEXT NOT NULL,
                PRIMARY KEY (name, version));
             CREATE TABLE IF NOT EXISTS memories (
                id INTEGER PRIMARY KEY AUTOINCREMENT, agent TEXT NOT NULL, key TEXT NOT NULL,
                content TEXT NOT NULL, tags TEXT, created_at TEXT NOT NULL);
             CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
                key, content, tags, content='memories', content_rowid='id');
             CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN
                INSERT INTO memories_fts(rowid, key, content, tags) VALUES (new.id, new.key, new.content, new.tags);
             END;",
        )?;
        // Migration: managers are employees who may staff and run their own crew.
        // Errors when the column already exists, which is the normal case.
        let _ = conn.execute("ALTER TABLE agents ADD COLUMN manager INTEGER NOT NULL DEFAULT 0", []);
        // Migration: a rating belongs to the model that earned it. Without this the
        // only measured signal per model is latency and failure rate, which favours
        // cheap fast models by construction and can never justify a better one.
        let _ = conn.execute("ALTER TABLE ratings ADD COLUMN model TEXT NOT NULL DEFAULT ''", []);
        // Migration: review routines — a routine can dispatch an AGENT with a
        // stored task on a schedule instead of running a shell command, so
        // judgment checks (page critiques, code audits) become durable
        // infrastructure instead of a remembered intention.
        let _ = conn.execute("ALTER TABLE routines ADD COLUMN agent TEXT NOT NULL DEFAULT ''", []);
        let _ = conn.execute("ALTER TABLE routines ADD COLUMN task TEXT NOT NULL DEFAULT ''", []);
        // Migration: objectives can declare what blocks them; blockedness is
        // derived at render time so it can never go stale.
        let _ = conn.execute("ALTER TABLE objectives ADD COLUMN blocked_by TEXT NOT NULL DEFAULT ''", []);
        // Migration: plans carry their own freshness stamp, so the board can
        // flag a plan that stopped moving while the objective kept advancing —
        // the signature of a pivot that edited notes but not the plan.
        let _ = conn.execute("ALTER TABLE objectives ADD COLUMN plan_updated_at TEXT NOT NULL DEFAULT ''", []);
        // Migration: objectives can have an owning manager; their workers'
        // reports route to the owner instead of the CEO.
        let _ = conn.execute("ALTER TABLE objectives ADD COLUMN owner TEXT NOT NULL DEFAULT ''", []);
        // Migration: objectives carry a category so the weekly portfolio review
        // can judge each lane by the right yardstick — before this, every lane
        // was implicitly measured like a profit center, which either kills the
        // social presence for earning nothing or lets "it's marketing" excuse
        // unlimited spend.
        let _ = conn.execute("ALTER TABLE objectives ADD COLUMN kind TEXT NOT NULL DEFAULT ''", []);
        Ok(Store { conn: Mutex::new(conn), log_tx: broadcast::channel(512).0 })
    }

    pub fn subscribe_log(&self) -> broadcast::Receiver<String> {
        self.log_tx.subscribe()
    }

    pub fn kv_get(&self, k: &str) -> Option<String> {
        let c = self.conn.lock().unwrap();
        c.query_row("SELECT v FROM kv WHERE k=?1", params![k], |r| r.get(0)).ok()
    }

    // --- CEO episodes (the transcript is disposable; notes carry continuity) ---

    pub fn add_episode(&self, started_at: &str, event_kind: &str, note: &str, steps: i64) {
        let c = self.conn.lock().unwrap();
        let _ = c.execute(
            "INSERT INTO episodes(started_at, ended_at, event_kind, note, steps) VALUES(?1,?2,?3,?4,?5)",
            params![started_at, chrono::Utc::now().to_rfc3339(), event_kind, note, steps],
        );
    }

    pub fn last_episode_note(&self) -> Option<String> {
        let c = self.conn.lock().unwrap();
        c.query_row("SELECT note FROM episodes ORDER BY id DESC LIMIT 1", [], |r| r.get(0)).ok()
    }

    /// One line per active employee for the episode brief.
    pub fn team_roster_text(&self) -> String {
        let c = self.conn.lock().unwrap();
        let Ok(mut stmt) = c.prepare(
            "SELECT name, model, manager FROM agents WHERE active=1 AND name!='CEO' ORDER BY name",
        ) else {
            return String::new();
        };
        let rows: Vec<(String, String, i64)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map(|it| it.filter_map(|x| x.ok()).collect())
            .unwrap_or_default();
        rows.iter()
            .map(|(n, m, mgr)| {
                format!("- {n} ({m}{})", if *mgr == 1 { ", manager" } else { "" })
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    // --- objective board (standing priority structure; immune to compaction) ---

    pub fn add_objective(&self, title: &str, rank: i64) -> i64 {
        let now = chrono::Utc::now().to_rfc3339();
        let c = self.conn.lock().unwrap();
        let _ = c.execute(
            "INSERT INTO objectives(title, rank, created_at, updated_at) VALUES(?1,?2,?3,?3)",
            params![title, rank, now],
        );
        c.last_insert_rowid()
    }

    /// Update any provided field; bumping updated_at counts as activity.
    pub fn update_objective(
        &self,
        id: i64,
        title: Option<&str>,
        rank: Option<i64>,
        plan: Option<&str>,
        note: Option<&str>,
        status: Option<&str>,
    ) -> bool {
        let now = chrono::Utc::now().to_rfc3339();
        let c = self.conn.lock().unwrap();
        let mut changed = 0;
        if let Some(v) = title {
            changed += c.execute("UPDATE objectives SET title=?2, updated_at=?3 WHERE id=?1", params![id, v, now]).unwrap_or(0);
        }
        if let Some(v) = rank {
            changed += c.execute("UPDATE objectives SET rank=?2, updated_at=?3 WHERE id=?1", params![id, v, now]).unwrap_or(0);
        }
        if let Some(v) = plan {
            changed += c
                .execute(
                    "UPDATE objectives SET plan=?2, updated_at=?3, plan_updated_at=?3 WHERE id=?1",
                    params![id, v, now],
                )
                .unwrap_or(0);
        }
        if let Some(v) = note {
            changed += c.execute("UPDATE objectives SET note=?2, updated_at=?3 WHERE id=?1", params![id, v, now]).unwrap_or(0);
        }
        if let Some(v) = status {
            changed += c.execute("UPDATE objectives SET status=?2, updated_at=?3 WHERE id=?1", params![id, v, now]).unwrap_or(0);
        }
        changed > 0
    }

    /// Test-only: backdate a plan's freshness stamp to exercise staleness.
    #[cfg(test)]
    pub fn backdate_plan(&self, id: i64, plan_updated_at: &str) {
        let c = self.conn.lock().unwrap();
        let _ = c.execute("UPDATE objectives SET plan_updated_at=?2 WHERE id=?1", params![id, plan_updated_at]);
    }

    /// Set an objective's portfolio category. Only the four known kinds are
    /// accepted — a free-text category would silently fall out of the weekly
    /// review's grouping.
    pub fn set_objective_kind(&self, id: i64, kind: &str) -> bool {
        if !["profit", "growth", "infra", "explore"].contains(&kind) {
            return false;
        }
        let now = chrono::Utc::now().to_rfc3339();
        let c = self.conn.lock().unwrap();
        c.execute("UPDATE objectives SET kind=?2, updated_at=?3 WHERE id=?1", params![id, kind, now])
            .unwrap_or(0)
            > 0
    }

    /// Assign (or clear, with "") the manager who owns an objective. Workers'
    /// reports on an owned objective route to the owner instead of the CEO.
    pub fn set_objective_owner(&self, id: i64, owner: &str) -> bool {
        let now = chrono::Utc::now().to_rfc3339();
        let c = self.conn.lock().unwrap();
        c.execute("UPDATE objectives SET owner=?2, updated_at=?3 WHERE id=?1", params![id, owner, now])
            .unwrap_or(0)
            > 0
    }

    /// Owner of an active objective, if it has one.
    pub fn objective_owner(&self, id: i64) -> Option<String> {
        let c = self.conn.lock().unwrap();
        c.query_row(
            "SELECT owner FROM objectives WHERE id=?1 AND status='active'",
            params![id],
            |r| r.get::<_, String>(0),
        )
        .ok()
        .filter(|o| !o.is_empty())
    }

    /// A manager left the company: their objectives revert to CEO routing.
    pub fn clear_objective_owner(&self, name: &str) -> usize {
        let c = self.conn.lock().unwrap();
        c.execute("UPDATE objectives SET owner='' WHERE owner=?1", params![name]).unwrap_or(0)
    }

    /// Set what an objective waits on. Normalized to digits-and-commas; empty clears.
    pub fn set_objective_blockers(&self, id: i64, blocked_by: &str) -> bool {
        let clean: String = blocked_by
            .split(',')
            .filter_map(|p| p.trim().trim_start_matches('#').parse::<i64>().ok())
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let now = chrono::Utc::now().to_rfc3339();
        let c = self.conn.lock().unwrap();
        c.execute("UPDATE objectives SET blocked_by=?2, updated_at=?3 WHERE id=?1", params![id, clean, now])
            .unwrap_or(0)
            > 0
    }

    /// Ids of the still-active blockers of one blocked_by string.
    fn unresolved(blocked_by: &str, active: &std::collections::HashSet<i64>) -> Vec<i64> {
        blocked_by
            .split(',')
            .filter_map(|p| p.trim().parse::<i64>().ok())
            .filter(|id| active.contains(id))
            .collect()
    }

    /// Active objectives whose LAST unresolved blocker is `done_id` — the ones a
    /// just-completed objective sets free. Call after marking `done_id` done.
    pub fn newly_ready(&self, done_id: i64) -> Vec<(i64, String)> {
        let c = self.conn.lock().unwrap();
        let Ok(mut stmt) = c.prepare("SELECT id, title, blocked_by FROM objectives WHERE status='active'") else {
            return vec![];
        };
        let rows: Vec<(i64, String, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map(|it| it.filter_map(|x| x.ok()).collect())
            .unwrap_or_default();
        let active: std::collections::HashSet<i64> = rows.iter().map(|(id, _, _)| *id).collect();
        rows.iter()
            .filter(|(_, _, b)| {
                let blockers: Vec<i64> = b.split(',').filter_map(|p| p.trim().parse().ok()).collect();
                // It was waiting on done_id, and nothing else still stands.
                blockers.contains(&done_id) && Self::unresolved(b, &active).is_empty()
            })
            .map(|(id, t, _)| (*id, t.clone()))
            .collect()
    }

    /// Stamp activity on an objective (a dispatch just went out against it).
    pub fn touch_objective(&self, id: i64) {
        let now = chrono::Utc::now().to_rfc3339();
        let c = self.conn.lock().unwrap();
        let _ = c.execute("UPDATE objectives SET updated_at=?2 WHERE id=?1", params![id, now]);
    }

    /// The board as shown to the CEO every iteration: active objectives by rank,
    /// with in-flight counts (passed in from the orchestrator) and how long since
    /// anything advanced each one. Facts only — the judgment is the model's job.
    pub fn objectives_board(&self, inflight: &std::collections::HashMap<i64, usize>) -> String {
        let c = self.conn.lock().unwrap();
        let Ok(mut stmt) = c.prepare(
            "SELECT id, title, rank, plan, note, blocked_by, updated_at, owner, COALESCE(plan_updated_at,'') FROM objectives
             WHERE status='active' ORDER BY rank, id",
        ) else {
            return String::new();
        };
        let rows: Vec<(i64, String, i64, String, String, String, String, String, String)> = stmt
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?))
            })
            .map(|it| it.filter_map(|x| x.ok()).collect())
            .unwrap_or_default();
        drop(stmt);
        let active: std::collections::HashSet<i64> = rows.iter().map(|r| r.0).collect();
        let title_of: std::collections::HashMap<i64, &str> =
            rows.iter().map(|r| (r.0, r.1.as_str())).collect();
        let now = chrono::Utc::now();
        let (mut ready, mut blocked) = (Vec::new(), Vec::new());
        for (id, title, rank, plan, note, blocked_by, updated, owner, plan_updated) in &rows {
            let waiting = Self::unresolved(blocked_by, &active);
            if waiting.is_empty() {
                // Warnings live only here: a blocked objective is exempt from
                // staffing pressure by construction.
                let mins = chrono::DateTime::parse_from_rfc3339(updated)
                    .map(|t| (now - t.with_timezone(&chrono::Utc)).num_minutes())
                    .unwrap_or(0);
                let busy = inflight.get(id).copied().unwrap_or(0);
                let mut line = format!(
                    "#{id} [rank {rank}] {title} — {busy} task(s) in flight, last advanced {mins}m ago"
                );
                if !owner.is_empty() {
                    line.push_str(&format!(" — owned by {owner}"));
                }
                if busy == 0 {
                    line.push_str(" — UNSTAFFED");
                }
                if plan.is_empty() {
                    line.push_str(" — NO PLAN YET");
                } else if let (Ok(p), Ok(u)) = (
                    chrono::DateTime::parse_from_rfc3339(plan_updated),
                    chrono::DateTime::parse_from_rfc3339(updated),
                ) {
                    // The pivot signature: work kept advancing for a day+ after
                    // the plan last moved. Either the plan is still right
                    // (touch it) or the bet pivoted (close it, open a
                    // successor, plan that fresh).
                    let lag = (u - p).num_hours();
                    if lag >= 24 {
                        line.push_str(&format!(" — PLAN STALE? (untouched {}d while work advanced)", lag / 24));
                    }
                }
                if !note.is_empty() {
                    line.push_str(&format!(" — note: {}", note.chars().take(120).collect::<String>()));
                }
                ready.push(line);
            } else {
                let deps = waiting
                    .iter()
                    .map(|d| format!("#{d} ({})", title_of.get(d).copied().unwrap_or("?").chars().take(50).collect::<String>()))
                    .collect::<Vec<_>>()
                    .join(", ");
                blocked.push(format!("#{id} [rank {rank}] {title} — waiting on {deps}"));
            }
        }
        let mut out = String::new();
        if !ready.is_empty() {
            out.push_str("READY — every one of these should have hands:\n");
            out.push_str(&ready.join("\n"));
        }
        if !blocked.is_empty() {
            if !out.is_empty() {
                out.push_str("\n");
            }
            out.push_str("BLOCKED — do not staff these; finish the blocker instead:\n");
            out.push_str(&blocked.join("\n"));
        }
        out
    }

    /// The weekly portfolio review body: active objectives grouped by category,
    /// each group judged by its own yardstick, with each lane's share of the
    /// company's recent attention. Attention is approximated from the run log:
    /// dispatches tag agents to objectives ("objective":N in the dispatch
    /// detail), and every thinking event an agent logged after that is
    /// attributed to their most recently dispatched objective. Coarse — an
    /// agent juggling two lanes books everything to the newer one — but it is
    /// measured from real activity, and the alternative (no attribution) let a
    /// lane consume half the company invisibly.
    pub fn portfolio_review_text(&self, since_ts: &str) -> String {
        let c = self.conn.lock().unwrap();
        let Ok(mut stmt) = c.prepare(
            "SELECT id, title, kind, owner, note FROM objectives WHERE status='active' ORDER BY rank, id",
        ) else {
            return String::new();
        };
        let objs: Vec<(i64, String, String, String, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
            .map(|it| it.filter_map(|x| x.ok()).collect())
            .unwrap_or_default();
        drop(stmt);
        if objs.is_empty() {
            return String::new();
        }
        // agent -> objective of their latest dispatch in the window (rows come
        // oldest-first, so later assignments overwrite earlier ones).
        let mut assigned: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
        if let Ok(mut stmt) = c.prepare(
            "SELECT detail FROM run_log WHERE ts > ?1 AND event='dispatch' ORDER BY id",
        ) {
            let details: Vec<String> = stmt
                .query_map(params![since_ts], |r| r.get(0))
                .map(|it| it.filter_map(|x| x.ok()).collect())
                .unwrap_or_default();
            for d in details {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&d) {
                    if let (Some(agent), Some(oid)) = (v["agent"].as_str(), v["objective"].as_i64()) {
                        assigned.insert(agent.to_string(), oid);
                    }
                }
            }
        }
        let mut turns_by_obj: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
        let mut total_turns: i64 = 0;
        if let Ok(mut stmt) = c.prepare(
            "SELECT agent, COUNT(*) FROM run_log WHERE ts > ?1 AND event='thinking' GROUP BY agent",
        ) {
            let counts: Vec<(String, i64)> = stmt
                .query_map(params![since_ts], |r| Ok((r.get(0)?, r.get(1)?)))
                .map(|it| it.filter_map(|x| x.ok()).collect())
                .unwrap_or_default();
            for (agent, n) in counts {
                total_turns += n;
                if let Some(oid) = assigned.get(&agent) {
                    *turns_by_obj.entry(*oid).or_default() += n;
                }
            }
        }
        let line = |(id, title, _k, owner, note): &(i64, String, String, String, String)| {
            let share = if total_turns > 0 {
                100 * turns_by_obj.get(id).copied().unwrap_or(0) / total_turns
            } else {
                0
            };
            let mut l = format!("- #{id} {title} — ~{share}% of the company's attention this period");
            if !owner.is_empty() {
                l.push_str(&format!(", owned by {owner}"));
            }
            if !note.is_empty() {
                l.push_str(&format!(" — {}", note.chars().take(100).collect::<String>()));
            }
            l
        };
        let mut out = String::new();
        for (kind, header) in [
            ("profit", "PROFIT LANES — the only lanes judged in dollars. For EACH: pull revenue booked vs fuel + attention spent from the books, name it a measured winner or loser, and kill or scale accordingly."),
            ("growth", "GROWTH / AUDIENCE — never judged on revenue. Judge cost per unit of attention and its TREND (followers, engagement, visits, donations). Kill only if audience is flat while spend continues; cap each lane's spend envelope."),
            ("infra", "INFRASTRUCTURE / OPS — a cost center you want. Judge reliability (alerts, failures) and cost trend. The question is never kill — it is: is this getting cheaper and quieter?"),
            ("explore", "EXPLORATION — judged on learning per dollar. A capped spend that produced a decisive, documented verdict is a WIN even when the verdict was no. Kill any exploration that is neither spending its cap nor producing verdicts."),
        ] {
            let lanes: Vec<String> = objs.iter().filter(|o| o.2 == kind).map(line).collect();
            if !lanes.is_empty() {
                out.push_str(&format!("\n{header}\n{}\n", lanes.join("\n")));
            }
        }
        let unclassified: Vec<String> = objs.iter().filter(|o| !["profit", "growth", "infra", "explore"].contains(&o.2.as_str())).map(line).collect();
        if !unclassified.is_empty() {
            out.push_str(&format!(
                "\nUNCLASSIFIED — these lanes are invisible to the portfolio until categorized. Set each with \
objectives(update, id, kind): profit (earns money), growth (buys audience), infra (keeps the company running), \
explore (buys knowledge).\n{}\n",
                unclassified.join("\n")
            ));
        }
        out
    }

    pub fn kv_set(&self, k: &str, v: &str) {
        let c = self.conn.lock().unwrap();
        let _ = c.execute("INSERT INTO kv(k,v) VALUES(?1,?2) ON CONFLICT(k) DO UPDATE SET v=?2", params![k, v]);
    }

    pub fn log(&self, agent: &str, event: &str, detail: &str) {
        let ts = chrono::Utc::now().to_rfc3339();
        // Scrub before the insert, so the viewer's history replay is covered too.
        let detail = redact(detail);
        let id = {
            let c = self.conn.lock().unwrap();
            match c.execute(
                "INSERT INTO run_log(ts, agent, event, detail) VALUES(?1,?2,?3,?4)",
                params![ts, agent, event, detail],
            ) {
                Ok(_) => c.last_insert_rowid(),
                Err(_) => 0,
            }
        };
        let _ = self.log_tx.send(log_row_json(id, &ts, agent, event, &detail));
    }

    /// Last n log rows as JSON (oldest first), same shape as the live stream.
    pub fn log_tail(&self, n: u32) -> Vec<String> {
        let c = self.conn.lock().unwrap();
        let mut stmt = match c.prepare(
            "SELECT id, ts, agent, event, detail FROM run_log ORDER BY id DESC LIMIT ?1",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        let mut rows: Vec<String> = stmt
            .query_map(params![n], |r| {
                let (id, ts, agent, event, detail): (i64, String, String, String, Option<String>) =
                    (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?);
                Ok(log_row_json(id, &ts, &agent, &event, &detail.unwrap_or_default()))
            })
            .map(|it| it.filter_map(|x| x.ok()).collect())
            .unwrap_or_default();
        rows.reverse();
        rows
    }

    pub fn recent_log(&self, n: u32) -> String {
        let c = self.conn.lock().unwrap();
        // The 12-second stats ticker would otherwise own the whole tail after
        // any quiet stretch — an episode brief of 15 JSON snapshots and no
        // actual activity.
        let mut stmt = match c.prepare(
            "SELECT ts, agent, event, detail FROM run_log WHERE event != 'stats' ORDER BY id DESC LIMIT ?1",
        ) {
            Ok(s) => s,
            Err(_) => return String::new(),
        };
        let rows: Vec<String> = stmt
            .query_map(params![n], |r| {
                let (ts, agent, event, detail): (String, String, String, Option<String>) =
                    (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?);
                Ok(format!("[{ts}] {agent} {event}: {}", detail.unwrap_or_default().chars().take(200).collect::<String>()))
            })
            .map(|it| it.filter_map(|x| x.ok()).collect())
            .unwrap_or_default();
        rows.into_iter().rev().collect::<Vec<_>>().join("\n")
    }

    // --- tool health (so broken tooling becomes visible, not silently retried) ---

    pub fn record_tool_call(&self, tool: &str, ok: bool, err: &str) {
        let c = self.conn.lock().unwrap();
        let _ = c.execute(
            "INSERT INTO tool_calls(ts, tool, ok, err) VALUES(?1,?2,?3,?4)",
            params![chrono::Utc::now().to_rfc3339(), tool, ok as i64, err],
        );
    }

    /// Recently-failing tools, worst first, for the reflection step.
    /// Empty when everything is healthy, so a healthy run adds no prompt noise.
    pub fn tool_health_text(&self) -> String {
        let c = self.conn.lock().unwrap();
        let mut stmt = match c.prepare(
            "SELECT tool, COUNT(*) AS calls, SUM(1-ok) AS fails,
                    (SELECT err FROM tool_calls e WHERE e.tool=t.tool AND e.ok=0 ORDER BY e.id DESC LIMIT 1)
             FROM tool_calls t
             WHERE t.id > (SELECT COALESCE(MAX(id),0) - 300 FROM tool_calls)
             GROUP BY t.tool HAVING fails > 0 ORDER BY fails DESC",
        ) {
            Ok(s) => s,
            Err(_) => return String::new(),
        };
        let rows: Vec<String> = stmt
            .query_map([], |r| {
                let (tool, calls, fails, err): (String, i64, i64, Option<String>) =
                    (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?);
                Ok(format!(
                    "- {tool}: {fails} of {calls} recent calls FAILED — last error: {}",
                    err.unwrap_or_default().chars().take(160).collect::<String>()
                ))
            })
            .map(|it| it.filter_map(|x| x.ok()).collect())
            .unwrap_or_default();
        rows.join("\n")
    }

    // --- model performance (so hiring runs on measured speed and reliability, not price alone) ---

    pub fn record_model_call(&self, model: &str, ms: u64, ok: bool, err: &str) {
        let c = self.conn.lock().unwrap();
        let _ = c.execute(
            "INSERT INTO model_calls(ts, model, ms, ok, err) VALUES(?1,?2,?3,?4,?5)",
            params![chrono::Utc::now().to_rfc3339(), model, ms as i64, ok as i64, err],
        );
    }

    /// Measured per-model performance over the recent window, busiest first, for
    /// the reflection step. Unlike tool health this always reports: a model being
    /// slow-but-working is exactly the signal a hiring decision needs.
    pub fn model_stats_text(&self) -> String {
        let c = self.conn.lock().unwrap();
        let mut stmt = match c.prepare(
            "SELECT model, COUNT(*) AS calls, SUM(1-ok) AS fails,
                    ROUND(AVG(ms)/1000.0,1), ROUND(MAX(ms)/1000.0,1),
                    SUM(CASE WHEN ms >= 60000 THEN 1 ELSE 0 END),
                    (SELECT err FROM model_calls e WHERE e.model=m.model AND e.ok=0 ORDER BY e.id DESC LIMIT 1)
             FROM model_calls m
             WHERE m.id > (SELECT COALESCE(MAX(id),0) - 500 FROM model_calls)
             GROUP BY m.model ORDER BY calls DESC",
        ) {
            Ok(s) => s,
            Err(_) => return String::new(),
        };
        let rows: Vec<String> = stmt
            .query_map([], |r| {
                let (model, calls, fails, avg_s, max_s, slow, err): (String, i64, i64, f64, f64, i64, Option<String>) =
                    (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?);
                let mut line = format!(
                    "- {model}: {calls} calls, avg {avg_s}s, worst {max_s}s, {slow} took 60s+, {fails} failed"
                );
                if let Some(e) = err.filter(|e| !e.is_empty()) {
                    line.push_str(&format!(" (last error: {})", e.chars().take(120).collect::<String>()));
                }
                Ok(line)
            })
            .map(|it| it.filter_map(|x| x.ok()).collect())
            .unwrap_or_default();
        rows.join("\n")
    }

    /// Every model with at least one recorded call.
    pub fn models_seen(&self) -> Vec<String> {
        let c = self.conn.lock().unwrap();
        let Ok(mut stmt) = c.prepare("SELECT DISTINCT model FROM model_calls") else {
            return Vec::new();
        };
        stmt.query_map([], |r| r.get::<_, String>(0))
            .map(|it| it.filter_map(|x| x.ok()).collect())
            .unwrap_or_default()
    }

    /// Average rated quality per model, with the sample size that earned it.
    ///
    /// The counterweight to model_stats_text. Latency and failure rate measure how
    /// fast a model answers and whether it errors — never whether the answer was
    /// any good — so on that evidence alone the cheapest flash model wins every
    /// comparison and no stronger model can ever be justified. Ratings are the
    /// CEO's own judgement rather than an objective score, so the sample count is
    /// shown alongside: one 5/5 is an anecdote, not a reason to re-home a team.
    pub fn model_quality_text(&self) -> String {
        let c = self.conn.lock().unwrap();
        let Ok(mut stmt) = c.prepare(
            "SELECT model, COUNT(*) n, ROUND(AVG(score),2) avg
             FROM ratings WHERE model != '' GROUP BY model ORDER BY avg DESC, n DESC",
        ) else {
            return String::new();
        };
        let rows: Vec<String> = stmt
            .query_map([], |r| {
                let (model, n, avg): (String, i64, f64) = (r.get(0)?, r.get(1)?, r.get(2)?);
                let confidence = if n < 3 { " — too few to trust yet" } else { "" };
                Ok(format!("- {model}: {avg}/5 over {n} rated task(s){confidence}"))
            })
            .map(|it| it.filter_map(|x| x.ok()).collect())
            .unwrap_or_default();
        rows.join("\n")
    }

    // --- routines (scheduled checks the binary runs itself — zero model cost) ---

    pub fn upsert_routine(&self, name: &str, command: &str, interval_secs: i64, purpose: &str) {
        let c = self.conn.lock().unwrap();
        let _ = c.execute(
            "INSERT INTO routines(name, command, interval_secs, purpose, enabled, last_run)
             VALUES(?1,?2,?3,?4,1,0)
             ON CONFLICT(name) DO UPDATE SET command=?2, interval_secs=?3, purpose=?4, enabled=1",
            params![name, command, interval_secs, purpose],
        );
    }

    pub fn delete_routine(&self, name: &str) -> bool {
        let c = self.conn.lock().unwrap();
        c.execute("DELETE FROM routines WHERE name=?1", params![name]).map(|n| n > 0).unwrap_or(false)
    }

    /// (name, command, interval_secs, purpose, last_status) for every routine.
    pub fn list_routines(&self) -> Vec<(String, String, i64, String, String)> {
        let c = self.conn.lock().unwrap();
        let mut stmt = match c.prepare(
            "SELECT name,
                    CASE WHEN COALESCE(agent,'') != '' THEN 'review by ' || agent || ': ' || substr(task,1,120) ELSE command END,
                    interval_secs, COALESCE(purpose,''), COALESCE(last_status,'never ran')
             FROM routines WHERE enabled=1 ORDER BY name",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
            .map(|it| it.filter_map(|x| x.ok()).collect())
            .unwrap_or_default()
    }

    /// Register a review routine: on schedule, `agent` is dispatched with
    /// `task` and their report flows back through normal routing. Same
    /// namespace as shell routines (same name = replace).
    pub fn upsert_review_routine(&self, name: &str, agent: &str, task: &str, interval_secs: i64, purpose: &str) {
        let c = self.conn.lock().unwrap();
        let _ = c.execute(
            "INSERT INTO routines(name, command, agent, task, interval_secs, purpose, enabled, last_run)
             VALUES(?1, '', ?2, ?3, ?4, ?5, 1, 0)
             ON CONFLICT(name) DO UPDATE SET command='', agent=?2, task=?3, interval_secs=?4, purpose=?5, enabled=1",
            params![name, agent, task, interval_secs, purpose],
        );
    }

    /// Routines whose interval has elapsed since their last run, as
    /// (name, command, agent, task) — command empty for review routines,
    /// agent empty for shell routines.
    pub fn due_routines(&self, now_epoch: i64) -> Vec<(String, String, String, String)> {
        let c = self.conn.lock().unwrap();
        let mut stmt = match c.prepare(
            "SELECT name, command, COALESCE(agent,''), COALESCE(task,'') FROM routines
             WHERE enabled=1 AND (last_run = 0 OR ?1 - last_run >= interval_secs) ORDER BY name",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map(params![now_epoch], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .map(|it| it.filter_map(|x| x.ok()).collect())
            .unwrap_or_default()
    }

    pub fn mark_routine_run(&self, name: &str, now_epoch: i64, status: &str) {
        let c = self.conn.lock().unwrap();
        let _ = c.execute(
            "UPDATE routines SET last_run=?2, last_status=?3 WHERE name=?1",
            params![name, now_epoch, status],
        );
    }

    pub fn add_routine_alert(&self, name: &str, detail: &str) {
        let c = self.conn.lock().unwrap();
        let _ = c.execute(
            "INSERT INTO routine_alerts(ts, name, detail) VALUES(?1,?2,?3)",
            params![chrono::Utc::now().to_rfc3339(), name, redact(detail)],
        );
    }

    /// Undelivered routine alerts, oldest first, as (name, detail); marks them delivered.
    pub fn drain_routine_alerts(&self) -> Vec<(String, String)> {
        let c = self.conn.lock().unwrap();
        let mut stmt = match c.prepare("SELECT id, name, detail FROM routine_alerts WHERE delivered=0 ORDER BY id") {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        let rows: Vec<(i64, String, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map(|it| it.filter_map(|x| x.ok()).collect())
            .unwrap_or_default();
        drop(stmt);
        for (id, _, _) in &rows {
            let _ = c.execute("UPDATE routine_alerts SET delivered=1 WHERE id=?1", params![id]);
        }
        rows.into_iter().map(|(_, n, d)| (n, d)).collect()
    }

    // --- founder messages (khan tell) ---

    pub fn queue_message(&self, msg: &str) {
        let c = self.conn.lock().unwrap();
        let _ = c.execute(
            "INSERT INTO messages(msg, created_at) VALUES(?1,?2)",
            params![msg, chrono::Utc::now().to_rfc3339()],
        );
    }

    // --- founder <-> CEO Telegram conversation (long-term chat memory) ---

    pub fn add_telegram_chat(&self, role: &str, text: &str) {
        let c = self.conn.lock().unwrap();
        let _ = c.execute(
            "INSERT INTO telegram_chat(ts, role, text) VALUES(?1,?2,?3)",
            params![chrono::Utc::now().to_rfc3339(), role, text],
        );
    }

    /// The last `n` exchanges, oldest first, as (role, text).
    pub fn telegram_tail(&self, n: usize) -> Vec<(String, String)> {
        let c = self.conn.lock().unwrap();
        let mut stmt = match c
            .prepare("SELECT role, text FROM (SELECT id, role, text FROM telegram_chat ORDER BY id DESC LIMIT ?1) ORDER BY id")
        {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map(params![n as i64], |r| Ok((r.get(0)?, r.get(1)?)))
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default()
    }

    /// Total stored chat size in characters (compaction trigger).
    pub fn telegram_chat_chars(&self) -> usize {
        let c = self.conn.lock().unwrap();
        c.query_row("SELECT IFNULL(SUM(LENGTH(text)),0) FROM telegram_chat", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap_or(0) as usize
    }

    /// Everything except the newest `keep` rows, oldest first, as
    /// (id, role, text) — the slice a compaction folds into the brief.
    pub fn telegram_old(&self, keep: usize) -> Vec<(i64, String, String)> {
        let c = self.conn.lock().unwrap();
        let mut stmt = match c.prepare(
            "SELECT id, role, text FROM telegram_chat \
             WHERE id NOT IN (SELECT id FROM telegram_chat ORDER BY id DESC LIMIT ?1) ORDER BY id",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map(params![keep as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default()
    }

    pub fn delete_telegram_upto(&self, id: i64) {
        let c = self.conn.lock().unwrap();
        let _ = c.execute("DELETE FROM telegram_chat WHERE id <= ?1", params![id]);
    }

    /// True when undelivered founder messages or routine alerts are queued —
    /// a cheap peek for the idle wait, which must never mark anything delivered.
    pub fn has_pending_input(&self) -> bool {
        let c = self.conn.lock().unwrap();
        let count = |sql: &str| c.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap_or(0);
        count("SELECT COUNT(*) FROM messages WHERE delivered=0") > 0
            || count("SELECT COUNT(*) FROM routine_alerts WHERE delivered=0") > 0
    }

    /// Undelivered founder messages, oldest first; marks them delivered.
    pub fn drain_messages(&self) -> Vec<String> {
        let c = self.conn.lock().unwrap();
        let mut stmt = match c.prepare("SELECT id, msg FROM messages WHERE delivered=0 ORDER BY id") {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        let rows: Vec<(i64, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map(|it| it.filter_map(|x| x.ok()).collect())
            .unwrap_or_default();
        drop(stmt);
        for (id, _) in &rows {
            let _ = c.execute("UPDATE messages SET delivered=1 WHERE id=?1", params![id]);
        }
        rows.into_iter().map(|(_, m)| m).collect()
    }

    // --- delegation ratings (ground truth for prompt evolution) ---

    pub fn add_rating(&self, agent: &str, score: i64, note: &str) {
        let c = self.conn.lock().unwrap();
        let pname = if agent == "CEO" { "CEO".to_string() } else { format!("agent:{agent}") };
        let pv: Option<i64> = c
            .query_row("SELECT MAX(version) FROM prompts WHERE name=?1", params![pname], |r| r.get(0))
            .ok()
            .flatten();
        // Stamp the model the agent was running when the work was rated: re-homing
        // changes it afterwards, and the score belongs to the model that earned it.
        let model: String = c
            .query_row("SELECT model FROM agents WHERE name=?1", params![agent], |r| r.get(0))
            .unwrap_or_default();
        let _ = c.execute(
            "INSERT INTO ratings(agent, score, note, prompt_version, created_at, model) VALUES(?1,?2,?3,?4,?5,?6)",
            params![agent, score, note, pv, chrono::Utc::now().to_rfc3339(), model],
        );
    }

    /// Per-agent rating stats for the reflection step.
    pub fn rating_stats_text(&self) -> String {
        let c = self.conn.lock().unwrap();
        let mut stmt = match c.prepare(
            "SELECT agent, COUNT(*), ROUND(AVG(score),2),
                    (SELECT ROUND(AVG(score),2) FROM (SELECT score FROM ratings r2 WHERE r2.agent=r.agent ORDER BY r2.id DESC LIMIT 5)),
                    (SELECT MAX(prompt_version) FROM ratings r3 WHERE r3.agent=r.agent)
             FROM ratings r GROUP BY agent ORDER BY agent",
        ) {
            Ok(s) => s,
            Err(_) => return String::new(),
        };
        let rows: Vec<String> = stmt
            .query_map([], |r| {
                let (agent, n, avg, avg5, pv): (String, i64, f64, Option<f64>, Option<i64>) =
                    (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?);
                Ok(format!(
                    "- {agent}: {n} rated tasks, avg {avg}/5, last-5 avg {}/5, current prompt v{}",
                    avg5.unwrap_or(avg), pv.unwrap_or(1)
                ))
            })
            .map(|it| it.filter_map(|x| x.ok()).collect())
            .unwrap_or_default();
        rows.join("\n")
    }

    // --- prompts (versioned) ---

    /// Insert a seed prompt only if the name doesn't exist yet.
    pub fn seed_prompt(&self, name: &str, content: &str) {
        let c = self.conn.lock().unwrap();
        let exists: bool = c
            .query_row("SELECT EXISTS(SELECT 1 FROM prompts WHERE name=?1)", params![name], |r| r.get(0))
            .unwrap_or(false);
        if !exists {
            let _ = c.execute(
                "INSERT INTO prompts(name, version, content, reason, created_at) VALUES(?1,1,?2,'seed',?3)",
                params![name, content, chrono::Utc::now().to_rfc3339()],
            );
        }
    }

    pub fn get_prompt(&self, name: &str) -> Option<String> {
        let c = self.conn.lock().unwrap();
        c.query_row(
            "SELECT content FROM prompts WHERE name=?1 ORDER BY version DESC LIMIT 1",
            params![name],
            |r| r.get(0),
        )
        .ok()
    }

    pub fn update_prompt(&self, name: &str, content: &str, reason: &str) -> Result<i64> {
        let c = self.conn.lock().unwrap();
        let next: i64 = c
            .query_row("SELECT COALESCE(MAX(version),0)+1 FROM prompts WHERE name=?1", params![name], |r| r.get(0))
            .unwrap_or(1);
        c.execute(
            "INSERT INTO prompts(name, version, content, reason, created_at) VALUES(?1,?2,?3,?4,?5)",
            params![name, next, content, reason, chrono::Utc::now().to_rfc3339()],
        )?;
        Ok(next)
    }

    pub fn rollback_prompt(&self, name: &str) -> Result<bool> {
        let c = self.conn.lock().unwrap();
        let n = c.execute(
            "DELETE FROM prompts WHERE name=?1 AND version=(SELECT MAX(version) FROM prompts WHERE name=?1)
             AND (SELECT COUNT(*) FROM prompts WHERE name=?1) > 1",
            params![name],
        )?;
        Ok(n > 0)
    }

    // --- custom tools (versioned like prompts) ---

    pub fn save_tool(&self, name: &str, description: &str, params: &str, lang: &str, script: &str, reason: &str) -> Result<i64> {
        let c = self.conn.lock().unwrap();
        let next: i64 = c
            .query_row("SELECT COALESCE(MAX(version),0)+1 FROM tool_defs WHERE name=?1", params![name], |r| r.get(0))
            .unwrap_or(1);
        c.execute(
            "INSERT INTO tool_defs(name, version, description, params, lang, script, reason, created_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![name, next, description, params, lang, script, reason, chrono::Utc::now().to_rfc3339()],
        )?;
        Ok(next)
    }

    /// Latest version of one tool: (description, params, lang, script).
    pub fn get_tool(&self, name: &str) -> Option<(String, String, String, String)> {
        let c = self.conn.lock().unwrap();
        c.query_row(
            "SELECT description, params, lang, script FROM tool_defs WHERE name=?1 ORDER BY version DESC LIMIT 1",
            params![name],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .ok()
    }

    /// Latest version of every tool: (name, description, params).
    pub fn list_tools(&self) -> Vec<(String, String, String)> {
        let c = self.conn.lock().unwrap();
        let mut stmt = match c.prepare(
            "SELECT name, description, params FROM tool_defs t
             WHERE version=(SELECT MAX(version) FROM tool_defs WHERE name=t.name) ORDER BY name",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map(|it| it.filter_map(|x| x.ok()).collect())
            .unwrap_or_default()
    }

    pub fn rollback_tool(&self, name: &str) -> Result<bool> {
        let c = self.conn.lock().unwrap();
        let n = c.execute(
            "DELETE FROM tool_defs WHERE name=?1 AND version=(SELECT MAX(version) FROM tool_defs WHERE name=?1)
             AND (SELECT COUNT(*) FROM tool_defs WHERE name=?1) > 1",
            params![name],
        )?;
        Ok(n > 0)
    }

    // --- skills (versioned procedural knowledge, like prompts/tools) ---

    pub fn save_skill(&self, name: &str, description: &str, content: &str, reason: &str) -> Result<i64> {
        let c = self.conn.lock().unwrap();
        let next: i64 = c
            .query_row("SELECT COALESCE(MAX(version),0)+1 FROM skill_defs WHERE name=?1", params![name], |r| r.get(0))
            .unwrap_or(1);
        c.execute(
            "INSERT INTO skill_defs(name, version, description, content, reason, created_at) VALUES(?1,?2,?3,?4,?5,?6)",
            params![name, next, description, content, reason, chrono::Utc::now().to_rfc3339()],
        )?;
        Ok(next)
    }

    /// Latest version of one skill: (description, content).
    pub fn get_skill(&self, name: &str) -> Option<(String, String)> {
        let c = self.conn.lock().unwrap();
        c.query_row(
            "SELECT description, content FROM skill_defs WHERE name=?1 ORDER BY version DESC LIMIT 1",
            params![name],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok()
    }

    /// Latest version of every skill: (name, description).
    pub fn list_skills(&self) -> Vec<(String, String)> {
        let c = self.conn.lock().unwrap();
        let mut stmt = match c.prepare(
            "SELECT name, description FROM skill_defs s
             WHERE version=(SELECT MAX(version) FROM skill_defs WHERE name=s.name) ORDER BY name",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map(|it| it.filter_map(|x| x.ok()).collect())
            .unwrap_or_default()
    }

    /// Latest version's provenance: (content, reason). The reason is how the
    /// seeder tells an untouched seed (reason starts with "seeded") from a
    /// version the company wrote itself.
    pub fn skill_latest_meta(&self, name: &str) -> Option<(String, String)> {
        let c = self.conn.lock().unwrap();
        c.query_row(
            "SELECT content, IFNULL(reason,'') FROM skill_defs WHERE name=?1 ORDER BY version DESC LIMIT 1",
            params![name],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok()
    }

    /// Permanently remove a skill (all versions). rollback_skill undoes one
    /// bad version; this is for a skill whose subject no longer exists.
    pub fn retire_skill(&self, name: &str) -> bool {
        let c = self.conn.lock().unwrap();
        c.execute("DELETE FROM skill_defs WHERE name=?1", params![name]).map(|n| n > 0).unwrap_or(false)
    }

    pub fn log_skill_load(&self, agent: &str, skill: &str) {
        let c = self.conn.lock().unwrap();
        let _ = c.execute(
            "INSERT INTO skill_loads(ts, agent, skill) VALUES(?1,?2,?3)",
            params![chrono::Utc::now().to_rfc3339(), agent, skill],
        );
    }

    /// Outcome stats for the reflection payload: each skill's loads over the
    /// last 30 days joined to the loading agent's NEXT rating within 24h (the
    /// task the load served), worst average first — plus, once the load log is
    /// two weeks deep, the skills nothing has loaded in 30 days. Empty string
    /// when there is nothing worth saying.
    pub fn skill_stats_text(&self) -> String {
        let c = self.conn.lock().unwrap();
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(30)).to_rfc3339();
        let mut lines: Vec<String> = Vec::new();
        if let Ok(mut stmt) = c.prepare(
            "SELECT l.skill, COUNT(*) AS loads, AVG(r.score) AS avg_score, COUNT(r.score) AS rated
             FROM skill_loads l
             LEFT JOIN ratings r ON r.id = (
                 SELECT id FROM ratings WHERE agent = l.agent AND created_at > l.ts
                   AND created_at < datetime(l.ts, '+1 day') ORDER BY id LIMIT 1)
             WHERE l.ts > ?1 GROUP BY l.skill ORDER BY avg_score IS NULL, avg_score LIMIT 10",
        ) {
            let rows = stmt
                .query_map(params![cutoff], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, Option<f64>>(2)?,
                        r.get::<_, i64>(3)?,
                    ))
                })
                .map(|it| it.filter_map(|x| x.ok()).collect::<Vec<_>>())
                .unwrap_or_default();
            for (skill, loads, avg, rated) in rows {
                match avg {
                    Some(a) if rated >= 3 => {
                        lines.push(format!("- {skill}: {loads} loads, avg outcome {a:.1}/5 over {rated} rated tasks"))
                    }
                    _ => lines.push(format!("- {skill}: {loads} loads, too few rated tasks to judge")),
                }
            }
        }
        // Unused list only once the load log is mature enough to mean something.
        let oldest: Option<String> =
            c.query_row("SELECT MIN(ts) FROM skill_loads", [], |r| r.get(0)).ok().flatten();
        let mature = oldest.is_some_and(|t| t < (chrono::Utc::now() - chrono::Duration::days(14)).to_rfc3339());
        if mature {
            if let Ok(mut stmt) = c.prepare(
                "SELECT name FROM skill_defs s WHERE version=(SELECT MAX(version) FROM skill_defs WHERE name=s.name)
                 AND name NOT IN (SELECT DISTINCT skill FROM skill_loads WHERE ts > ?1) ORDER BY name LIMIT 15",
            ) {
                let unused: Vec<String> = stmt
                    .query_map(params![cutoff], |r| r.get::<_, String>(0))
                    .map(|it| it.filter_map(|x| x.ok()).collect())
                    .unwrap_or_default();
                if !unused.is_empty() {
                    lines.push(format!(
                        "- UNLOADED 30d (candidates to retire_skill or merge — every index line is paid for each turn): {}",
                        unused.join(", ")
                    ));
                }
            }
        }
        lines.join("\n")
    }

    pub fn rollback_skill(&self, name: &str) -> Result<bool> {
        let c = self.conn.lock().unwrap();
        let n = c.execute(
            "DELETE FROM skill_defs WHERE name=?1 AND version=(SELECT MAX(version) FROM skill_defs WHERE name=?1)
             AND (SELECT COUNT(*) FROM skill_defs WHERE name=?1) > 1",
            params![name],
        )?;
        Ok(n > 0)
    }

    // --- agents ---

    pub fn save_agent(&self, name: &str, role: &str, prompt_name: &str, model: &str, history_json: &str) {
        let c = self.conn.lock().unwrap();
        let _ = c.execute(
            "INSERT INTO agents(name, role, prompt_name, model, history, active) VALUES(?1,?2,?3,?4,?5,1)
             ON CONFLICT(name) DO UPDATE SET role=?2, prompt_name=?3, model=?4, history=?5, active=1",
            params![name, role, prompt_name, model, history_json],
        );
    }

    pub fn load_agent(&self, name: &str) -> Option<(String, String, String, String)> {
        let c = self.conn.lock().unwrap();
        c.query_row(
            "SELECT role, prompt_name, model, history FROM agents WHERE name=?1 AND active=1",
            params![name],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .ok()
    }

    /// Mark an employee as a manager (may hire and run their own crew) or not.
    pub fn set_manager(&self, name: &str, manager: bool) {
        let c = self.conn.lock().unwrap();
        let _ = c.execute("UPDATE agents SET manager=?2 WHERE name=?1", params![name, manager as i64]);
    }

    pub fn is_manager(&self, name: &str) -> bool {
        let c = self.conn.lock().unwrap();
        c.query_row("SELECT manager FROM agents WHERE name=?1 AND active=1", params![name], |r| {
            r.get::<_, i64>(0)
        })
        .map(|m| m == 1)
        .unwrap_or(false)
    }

    /// Active employees, excluding the CEO — the number a hiring cap applies to.
    /// Headcount against the ceiling, and how long each employee has been silent.
    ///
    /// Nothing else measures this. Ratings say how well an employee works and the
    /// model stats say how fast, but neither says whether anyone is working at
    /// all — so a company can sit at four people with thirty-six seats free and
    /// read as healthy. Reported as data next to the other measured blocks,
    /// because measurement is what changed model choice when instructions did not.
    pub fn team_capacity_text(&self, ceiling: i64) -> String {
        let c = self.conn.lock().unwrap();
        let Ok(mut stmt) = c.prepare(
            "SELECT a.name, (SELECT MAX(ts) FROM run_log WHERE agent = a.name)
             FROM agents a WHERE a.active=1 AND a.name!='CEO' ORDER BY a.name",
        ) else {
            return String::new();
        };
        let rows: Vec<(String, Option<String>)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map(|it| it.flatten().collect())
            .unwrap_or_default();
        if rows.is_empty() {
            return format!("No employees at all (ceiling {ceiling}). Every task is yours by default.");
        }
        let now = chrono::Utc::now();
        let lines: Vec<String> = rows
            .iter()
            .map(|(name, last)| {
                match last
                    .as_deref()
                    .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                    .map(|t| (now - t.with_timezone(&chrono::Utc)).num_minutes().max(0))
                {
                    Some(m) => format!("  {name}: silent {m}m"),
                    None => format!("  {name}: has never done anything"),
                }
            })
            .collect();
        // When work was last STARTED through someone else, which needs no
        // judgement about what counts as progress: either the CEO has handed out
        // new work recently or it has been doing everything itself.
        let started: Option<String> = c
            .query_row(
                "SELECT MAX(ts) FROM run_log WHERE event IN ('dispatch','delegate','delegate_parallel','hire')",
                [],
                |r| r.get(0),
            )
            .ok()
            .flatten();
        let now = chrono::Utc::now();
        let last_start = match started
            .as_deref()
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            .map(|t| (now - t.with_timezone(&chrono::Utc)).num_minutes().max(0))
        {
            Some(m) => format!("\nLast time you started new work through anyone (dispatch/delegate/hire): {m}m ago."),
            None => "\nYou have never started work through anyone — everything so far has been you.".to_string(),
        };
        format!(
            "{} employees, ceiling {ceiling} — {} seats free:\n{}{last_start}",
            rows.len(),
            (ceiling - rows.len() as i64).max(0),
            lines.join("\n")
        )
    }

    pub fn count_active_agents(&self) -> i64 {
        let c = self.conn.lock().unwrap();
        c.query_row("SELECT COUNT(*) FROM agents WHERE active=1 AND name!='CEO'", [], |r| r.get(0))
            .unwrap_or(0)
    }

    pub fn fire_agent(&self, name: &str) -> bool {
        let c = self.conn.lock().unwrap();
        c.execute("UPDATE agents SET active=0 WHERE name=?1", params![name]).map(|n| n > 0).unwrap_or(false)
    }

    pub fn list_agents(&self) -> Vec<(String, String, String)> {
        let c = self.conn.lock().unwrap();
        let mut stmt = match c.prepare("SELECT name, role, model FROM agents WHERE active=1 AND name!='CEO'") {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map(|it| it.filter_map(|x| x.ok()).collect())
            .unwrap_or_default()
    }

    // --- memories ---

    pub fn remember(&self, agent: &str, key: &str, content: &str, tags: &str) {
        let c = self.conn.lock().unwrap();
        let _ = c.execute(
            "INSERT INTO memories(agent, key, content, tags, created_at) VALUES(?1,?2,?3,?4,?5)",
            params![agent, key, content, tags, chrono::Utc::now().to_rfc3339()],
        );
    }

    pub fn recall(&self, query: &str, limit: u32) -> Vec<String> {
        let c = self.conn.lock().unwrap();
        // Sanitize into simple OR-joined FTS terms to avoid syntax errors from raw input.
        let terms: Vec<String> = query
            .split(|ch: char| !ch.is_alphanumeric())
            .filter(|t| t.len() > 2)
            .take(12)
            .map(|t| format!("\"{}\"", t))
            .collect();
        if terms.is_empty() {
            return vec![];
        }
        let fts = terms.join(" OR ");
        let mut stmt = match c.prepare(
            "SELECT m.key, m.content FROM memories_fts f JOIN memories m ON m.id=f.rowid
             WHERE memories_fts MATCH ?1 ORDER BY rank LIMIT ?2",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        let mut hits: Vec<String> = stmt
            .query_map(params![fts, limit], |r| {
                let (k, v): (String, String) = (r.get(0)?, r.get(1)?);
                Ok(format!("[{k}] {v}"))
            })
            .map(|it| it.filter_map(|x| x.ok()).collect())
            .unwrap_or_default();
        // Skill bodies are institutional knowledge too — a fact buried in a
        // skill an agent never loads is invisible to recall, which is how a
        // scout re-derived a fee-change premise its own copy skill had already
        // debunked (and the CEO rated it 5/5 with the debunk a table away).
        // Scan the latest version of every skill for the same terms and append
        // the matching lines, so the contradiction rides into context wherever
        // the claim goes. ~50 small docs: a full scan is cheaper than keeping
        // an FTS mirror consistent across versions and retires.
        let lterms: Vec<String> = terms.iter().map(|t| t.trim_matches('"').to_lowercase()).collect();
        if let Ok(mut sk) = c.prepare(
            "SELECT name, content FROM skill_defs s1
             WHERE version=(SELECT max(version) FROM skill_defs s2 WHERE s2.name=s1.name)",
        ) {
            let mut scored: Vec<(usize, String)> = sk
                .query_map([], |r| {
                    let (name, content): (String, String) = (r.get(0)?, r.get(1)?);
                    Ok((name, content))
                })
                .map(|it| it.filter_map(|x| x.ok()).collect::<Vec<_>>())
                .unwrap_or_default()
                .into_iter()
                .filter_map(|(name, content)| {
                    let lc = content.to_lowercase();
                    let found = lterms.iter().filter(|t| lc.contains(t.as_str())).count();
                    // One shared term across 50 docs is noise; demand overlap.
                    if found < 2 {
                        return None;
                    }
                    let lines: Vec<&str> = content
                        .lines()
                        .filter(|l| {
                            let ll = l.to_lowercase();
                            lterms.iter().filter(|t| ll.contains(t.as_str())).count() >= 2
                        })
                        .take(3)
                        .collect();
                    if lines.is_empty() {
                        return None;
                    }
                    let mut excerpt = lines.join(" / ");
                    // Cut on a char boundary: String::truncate panics mid-char,
                    // and skill bodies are full of multi-byte punctuation — a
                    // bad cut here crash-looped the whole binary (2026-08-31).
                    if excerpt.len() > 400 {
                        let mut cut = 400;
                        while !excerpt.is_char_boundary(cut) {
                            cut -= 1;
                        }
                        excerpt.truncate(cut);
                    }
                    Some((found, format!("[skill {name} — load it for the full picture] {excerpt}")))
                })
                .collect();
            scored.sort_by(|a, b| b.0.cmp(&a.0));
            hits.extend(scored.into_iter().take(3).map(|(_, s)| s));
        }
        hits
    }
}
