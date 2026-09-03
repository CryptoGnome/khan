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

/// Whether a stored prompt can run an agent: real text, not a pointer.
pub fn prompt_usable(content: &str) -> bool {
    let t = content.trim();
    t.chars().count() >= 400 && !(t.starts_with("http://") || t.starts_with("https://"))
}

/// What the ledger says about each objective, computed here rather than
/// written by the CEO: the net of every workspace `pnl` row whose note tags
/// the objective ("obj 5", "obj#5", "objective 5"), per asset. The trend-launch
/// lane ran eight launches at an identical loss between 2026-08-31 and
/// 2026-09-02 and stayed open, because no board line ever showed the tally.
/// Tagged rows alone misled within the hour of shipping: objective 5's
/// exits were tagged and its dev buys were not, so the line read +0.43 SOL
/// on a lane that had lost 0.22. So the tally also follows the lane's own
/// tickers — every asset named in a tagged row — into `closed_positions`,
/// where entry and exit sit on one row per trade. Deliberate ceiling: the
/// link is the ticker name in the note, and a trade whose ticker never
/// appears in a tagged row stays invisible.
pub fn lane_ledger(workspace: &std::path::Path) -> std::collections::HashMap<i64, String> {
    use std::collections::{BTreeMap, HashMap};
    let mut out = HashMap::new();
    let path = workspace.join("workspace.db");
    let Ok(c) = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY) else {
        return out;
    };
    let Ok(mut stmt) = c.prepare("SELECT COALESCE(category,''), COALESCE(asset,''), COALESCE(amount,0), COALESCE(note,'') FROM pnl") else {
        return out;
    };
    let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, f64>(2)?, r.get::<_, String>(3)?))) else {
        return out;
    };
    // (ticker -> (closed trades, winners, net SOL)) from the trade table.
    let mut closed: HashMap<String, (usize, usize, f64)> = HashMap::new();
    if let Ok(mut st) = c.prepare("SELECT COALESCE(asset,''), COALESCE(net_sol,0) FROM closed_positions") {
        if let Ok(rs) = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?))) {
            for (asset, net) in rs.flatten() {
                let e = closed.entry(asset.to_uppercase()).or_default();
                e.0 += 1;
                e.1 += usize::from(net > 0.0);
                e.2 += net;
            }
        }
    }
    let mut per: HashMap<i64, (usize, BTreeMap<String, f64>, std::collections::BTreeSet<String>)> = HashMap::new();
    for (category, asset, amount, note) in rows.flatten() {
        let Some(id) = tagged_objective(&note) else { continue };
        let e = per.entry(id).or_default();
        // The lane's tickers: any word in a tagged note that is a traded asset.
        for w in note.split(|ch: char| !ch.is_ascii_alphanumeric()) {
            let up = w.to_uppercase();
            if up.len() >= 3 && closed.contains_key(&up) {
                e.2.insert(up);
            }
        }
        if category == "bookkeeping" || amount == 0.0 || asset.is_empty() {
            continue;
        }
        e.0 += 1;
        *e.1.entry(asset).or_insert(0.0) += amount;
    }
    for (id, (n, assets, tickers)) in per {
        let nets = assets
            .iter()
            .map(|(a, v)| format!("{v:+.4} {a}"))
            .collect::<Vec<_>>()
            .join(", ");
        let mut line = format!(" — LEDGER (pnl rows tagged #{id}): {n} rows, net {nets}");
        if !tickers.is_empty() {
            let (mut t, mut won, mut net) = (0, 0, 0.0);
            for k in &tickers {
                let (a, b, c) = closed[k];
                t += a;
                won += b;
                net += c;
            }
            line.push_str(&format!(
                "; closed trades on its tickers ({}): {t} closed, {won} won, net {net:+.4} SOL",
                tickers.iter().cloned().collect::<Vec<_>>().join(", ")
            ));
        }
        out.insert(id, line);
    }
    out
}

/// The objective a ledger note tags: "obj 5", "obj#5", "obj #5", "objective 5".
pub fn tagged_objective(note: &str) -> Option<i64> {
    let lower = note.to_lowercase();
    for key in ["objective", "obj"] {
        let mut from = 0;
        while let Some(i) = lower[from..].find(key) {
            let start = from + i;
            let before_ok = start == 0 || !lower.as_bytes()[start - 1].is_ascii_alphanumeric();
            let rest = &lower[start + key.len()..];
            let rest = rest.trim_start_matches(|c: char| c == ' ' || c == '#' || c == ':');
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if before_ok && !digits.is_empty() {
                return digits.parse().ok();
            }
            from = start + key.len();
        }
    }
    None
}

/// (device, inode) of the file at `path`; None off Unix or for ":memory:".
fn file_identity(path: &str) -> Option<(u64, u64)> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = std::fs::metadata(path).ok()?;
        Some((m.dev(), m.ino()))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

/// Internal persistent state (khan.db): prompts, agents, run log, memories, kv.
pub struct Store {
    pub conn: Mutex<Connection>,
    /// Live feed of run_log rows as JSON, consumed by the web log viewer.
    log_tx: broadcast::Sender<String>,
    /// The path opened and the inode found there, so a later check can tell
    /// whether the file at the path is still the one this connection holds.
    opened: Option<(String, (u64, u64))>,
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
             CREATE TABLE IF NOT EXISTS dispatches (
                id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT NOT NULL, agent TEXT NOT NULL,
                objective INTEGER, class TEXT NOT NULL, shape TEXT NOT NULL);
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
             CREATE TABLE IF NOT EXISTS x_ledger (
                id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT NOT NULL,
                kind TEXT NOT NULL, amount_usd REAL NOT NULL, detail TEXT NOT NULL);
             INSERT INTO x_ledger(ts, kind, amount_usd, detail)
                SELECT datetime('now'), 'topup', 5.0, 'founder seed'
                WHERE NOT EXISTS (SELECT 1 FROM x_ledger);
             CREATE TABLE IF NOT EXISTS x_seen (
                rid TEXT NOT NULL, day TEXT NOT NULL, PRIMARY KEY (rid, day));
             CREATE TABLE IF NOT EXISTS x_replied (
                tweet_id TEXT PRIMARY KEY, ts TEXT NOT NULL, our_id TEXT NOT NULL);
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
        // Migration: an objective answers for its own date the way a revenue
        // idea does. Eight lanes opened 2026-08-29..31 were all still active on
        // 09-02 with 623 dispatches spread across them and nothing closing —
        // nothing ever asked one to justify staying open.
        let _ = conn.execute("ALTER TABLE objectives ADD COLUMN review_date TEXT NOT NULL DEFAULT ''", []);
        let _ = conn.execute("ALTER TABLE objectives ADD COLUMN kill_criterion TEXT NOT NULL DEFAULT ''", []);
        // Migration: what a call cost. The catalog's average price is what the
        // ladder read for a week while the models page showed luna's best ask
        // at a twentieth of glm53flash's (2026-09-02); the fill's own settled
        // charge is the price we pay, and only a record of it can move seats.
        for col in ["prompt_tokens", "completion_tokens", "micros"] {
            let _ = conn.execute(&format!("ALTER TABLE model_calls ADD COLUMN {col} INTEGER NOT NULL DEFAULT 0"), []);
        }
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
        // Migration: routines can have an owning agent; a shell routine's
        // ALERT then dispatches the owner to handle it instead of waking the
        // CEO. Structural org design: the CEO was the sole inbox for every
        // alert, so it self-triaged books drift by hand (2026-08-31) instead
        // of a finance owner handling it in parallel.
        let _ = conn.execute("ALTER TABLE routines ADD COLUMN owner TEXT NOT NULL DEFAULT ''", []);
        // A founder message used to be consumed as a one-time event: if the
        // CEO did not finish acting on it inside that episode, the intent
        // was gone. Delivered-but-unacknowledged messages now stand in the
        // brief until the CEO acks them.
        // The column lands once; everything delivered before it existed was
        // handled or lost under the old rules, and resurrecting every khan
        // tell ever sent would bury the brief. Only messages delivered from
        // here on stand until acked.
        if conn.execute("ALTER TABLE messages ADD COLUMN acked INTEGER NOT NULL DEFAULT 0", []).is_ok() {
            let _ = conn.execute("UPDATE messages SET acked=1 WHERE delivered=1", []);
        }
        let opened = file_identity(path).map(|id| (path.to_string(), id));
        Ok(Store { conn: Mutex::new(conn), log_tx: broadcast::channel(512).0, opened })
    }

    /// Why the database file is no longer the one this store opened, if so.
    ///
    /// An agent swapped the live file on 2026-09-03: `VACUUM INTO` a copy,
    /// rename the original aside, move the copy into place. The running
    /// binary kept its handle on the renamed original, its write-ahead log was
    /// applied to the wrong file, and by 04:26Z it saw an empty company while
    /// the site read a frozen snapshot — for seven hours, with no error
    /// anywhere. A connection to a file nobody else can see is worse than no
    /// process at all; the caller exits and lets the platform restart on the
    /// real path.
    pub fn file_replaced(&self) -> Option<String> {
        let (path, at_open) = self.opened.as_ref()?;
        match file_identity(path) {
            Some(now) if now == *at_open => None,
            Some(_) => Some(format!("{path} is a different file from the one opened at start")),
            None => Some(format!("{path} is gone from disk")),
        }
    }

    pub fn subscribe_log(&self) -> broadcast::Receiver<String> {
        self.log_tx.subscribe()
    }

    /// Startups logged within the last `window_secs` — the crash-loop
    /// tripwire reads this right after logging its own startup row.
    pub fn recent_startup_count(&self, window_secs: i64) -> i64 {
        let since = (chrono::Utc::now() - chrono::Duration::seconds(window_secs)).to_rfc3339();
        let c = self.conn.lock().unwrap();
        c.query_row(
            "SELECT count(*) FROM run_log WHERE event='startup' AND ts > ?1",
            params![since],
            |r| r.get(0),
        )
        .unwrap_or(0)
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

    /// Set an objective's review date (YYYY-MM-DD) and the condition that
    /// would kill it. Either may be empty to leave it alone.
    pub fn set_objective_review(&self, id: i64, review_date: &str, kill_criterion: &str) -> bool {
        let now = chrono::Utc::now().to_rfc3339();
        let c = self.conn.lock().unwrap();
        let mut changed = 0;
        if !review_date.is_empty() {
            changed += c
                .execute("UPDATE objectives SET review_date=?2, updated_at=?3 WHERE id=?1", params![id, review_date, now])
                .unwrap_or(0);
        }
        if !kill_criterion.is_empty() {
            changed += c
                .execute("UPDATE objectives SET kill_criterion=?2, updated_at=?3 WHERE id=?1", params![id, kill_criterion, now])
                .unwrap_or(0);
        }
        changed > 0
    }

    /// Active objectives whose own review date has passed, oldest first, plus
    /// the ones that never got a date at all (reported with an empty date).
    /// (id, title, review_date, kill_criterion)
    /// Active objectives that owe a decision: no review time, a review time
    /// that has passed, or one past `horizon` (a shelf, not a commitment).
    /// Review strings are `YYYY-MM-DD` or `YYYY-MM-DDTHH:MMZ`; both compare
    /// correctly as text against a `now` in the second form. `horizon` empty
    /// disables the shelf check. The fifth field is true for a shelved one.
    pub fn overdue_objectives(&self, now: &str, horizon: &str) -> Vec<(i64, String, String, String, bool)> {
        let c = self.conn.lock().unwrap();
        let Ok(mut stmt) = c.prepare(
            "SELECT id, title, COALESCE(review_date,''), COALESCE(kill_criterion,''), \
                    (?2 != '' AND review_date > ?2) FROM objectives \
             WHERE status='active' AND (COALESCE(review_date,'')='' OR review_date <= ?1 OR (?2 != '' AND review_date > ?2)) \
             ORDER BY COALESCE(NULLIF(review_date,''), '0000-00-00'), id",
        ) else {
            return Vec::new();
        };
        let rows = stmt
            .query_map(params![now, horizon], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
            .map(|it| it.filter_map(|x| x.ok()).collect())
            .unwrap_or_default();
        rows
    }

    /// Active objectives ordered by how soon they come up for review, for the
    /// refusal that tells the CEO what it would have to close first.
    pub fn active_objectives_by_review_date(&self) -> Vec<(i64, String, String)> {
        let c = self.conn.lock().unwrap();
        let Ok(mut stmt) = c.prepare(
            "SELECT id, title, COALESCE(review_date,'') FROM objectives WHERE status='active' \
             ORDER BY COALESCE(NULLIF(review_date,''), '9999-12-31'), id",
        ) else {
            return Vec::new();
        };
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map(|it| it.filter_map(|x| x.ok()).collect())
            .unwrap_or_default()
    }

    /// An objective's review date, or "" when it has none.
    pub fn objective_review_date(&self, id: i64) -> String {
        let c = self.conn.lock().unwrap();
        c.query_row("SELECT COALESCE(review_date,'') FROM objectives WHERE id=?1", params![id], |r| r.get(0))
            .unwrap_or_default()
    }

    /// The classes of an objective's most recent dispatches, newest first.
    pub fn recent_dispatch_classes(&self, objective: i64, n: usize) -> Vec<String> {
        let c = self.conn.lock().unwrap();
        let Ok(mut stmt) =
            c.prepare("SELECT class FROM dispatches WHERE objective=?1 ORDER BY id DESC LIMIT ?2")
        else {
            return Vec::new();
        };
        stmt.query_map(params![objective, n as i64], |r| r.get::<_, String>(0))
            .map(|it| it.filter_map(|x| x.ok()).collect())
            .unwrap_or_default()
    }

    /// An objective's portfolio category, or "" when it has none.
    pub fn objective_kind(&self, id: i64) -> String {
        let c = self.conn.lock().unwrap();
        c.query_row("SELECT COALESCE(kind,'') FROM objectives WHERE id=?1", params![id], |r| r.get(0))
            .unwrap_or_default()
    }

    /// Set an objective's portfolio category. Only the four known kinds are
    /// accepted — a free-text category would silently fall out of the weekly
    /// review's grouping.
    pub fn set_objective_kind(&self, id: i64, kind: &str) -> bool {
        if !["profit", "growth", "infra", "explore", "ops"].contains(&kind) {
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
    // --- dispatch accounting: what the company's hands are doing, by kind ---
    //
    // On 2026-09-01, 188 of 384 dispatches were checks of earlier work, the
    // same task shape went out four times, and 173 carried no objective. The
    // classification is a leading-verb heuristic (agent.rs classify_task);
    // the table is what the budget, the repeat refusal and the board read.

    pub fn record_dispatch(&self, agent: &str, objective: Option<i64>, class: &str, shape: &str) {
        let c = self.conn.lock().unwrap();
        let _ = c.execute(
            "INSERT INTO dispatches(ts, agent, objective, class, shape) VALUES(?1,?2,?3,?4,?5)",
            params![chrono::Utc::now().to_rfc3339(), agent, objective, class, shape],
        );
    }

    /// Check-class dispatches on an objective since its last build-class one
    /// (all of them, if nothing was ever built).
    pub fn consecutive_checks(&self, objective: i64) -> u32 {
        let c = self.conn.lock().unwrap();
        let mut stmt = match c.prepare(
            "SELECT class FROM dispatches WHERE objective=?1 ORDER BY id DESC LIMIT 20",
        ) {
            Ok(s) => s,
            Err(_) => return 0,
        };
        let classes: Vec<String> = stmt
            .query_map(params![objective], |r| r.get(0))
            .map(|it| it.flatten().collect())
            .unwrap_or_default();
        let mut n = 0;
        for cl in classes {
            match cl.as_str() {
                "check" => n += 1,
                "build" => break,
                _ => {}
            }
        }
        n
    }

    /// How many times this task shape went out in the last 24h.
    pub fn shape_count_24h(&self, shape: &str) -> u32 {
        let since = (chrono::Utc::now() - chrono::Duration::hours(24)).to_rfc3339();
        let c = self.conn.lock().unwrap();
        c.query_row(
            "SELECT COUNT(*) FROM dispatches WHERE shape=?1 AND ts>?2",
            params![shape, since],
            |r| r.get::<_, u32>(0),
        )
        .unwrap_or(0)
    }

    /// Per objective, (build, check) dispatch counts over the last 24h.
    pub fn objective_mix_24h(&self) -> std::collections::HashMap<i64, (u32, u32)> {
        let since = (chrono::Utc::now() - chrono::Duration::hours(24)).to_rfc3339();
        let c = self.conn.lock().unwrap();
        let mut out = std::collections::HashMap::new();
        let Ok(mut stmt) = c.prepare(
            "SELECT objective, class, COUNT(*) FROM dispatches WHERE ts>?1 AND objective IS NOT NULL GROUP BY objective, class",
        ) else {
            return out;
        };
        if let Ok(rows) = stmt.query_map(params![since], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, u32>(2)?))) {
            for (o, cl, n) in rows.flatten() {
                let e = out.entry(o).or_insert((0, 0));
                match cl.as_str() {
                    "build" => e.0 += n,
                    "check" => e.1 += n,
                    _ => {}
                }
            }
        }
        out
    }

    pub fn touch_objective(&self, id: i64) {
        let now = chrono::Utc::now().to_rfc3339();
        let c = self.conn.lock().unwrap();
        let _ = c.execute("UPDATE objectives SET updated_at=?2 WHERE id=?1", params![id, now]);
    }

    /// How many objectives are live right now — feeds the idle-capacity line
    /// the CEO sees each iteration, so "everything is owned" can't close an
    /// episode while the roster sits idle under an open board.
    pub fn active_objective_count(&self) -> usize {
        let c = self.conn.lock().unwrap();
        c.query_row("SELECT COUNT(*) FROM objectives WHERE status='active'", [], |r| r.get::<_, i64>(0))
            .map(|n| n as usize)
            .unwrap_or(0)
    }

    /// The board as shown to the CEO every iteration: active objectives by rank,
    /// with in-flight counts (passed in from the orchestrator) and how long since
    /// anything advanced each one. Facts only — the judgment is the model's job.
    pub fn objectives_board(&self, inflight: &std::collections::HashMap<i64, usize>) -> String {
        self.objectives_board_with(inflight, &std::collections::HashMap::new(), "")
    }

    /// The board with what the binary knows and the CEO cannot write: a
    /// per-objective suffix (`extra`: the ledger tally, an ops lane's routine
    /// status) and the review horizon, past which a date reads as a shelf.
    pub fn objectives_board_with(
        &self,
        inflight: &std::collections::HashMap<i64, usize>,
        extra: &std::collections::HashMap<i64, String>,
        horizon: &str,
    ) -> String {
        let mix = self.objective_mix_24h();
        let c = self.conn.lock().unwrap();
        let Ok(mut stmt) = c.prepare(
            "SELECT id, title, rank, plan, note, blocked_by, updated_at, owner, COALESCE(plan_updated_at,''), COALESCE(kind,''), COALESCE(review_date,'') FROM objectives
             WHERE status='active' ORDER BY rank, id",
        ) else {
            return String::new();
        };
        let rows: Vec<(i64, String, i64, String, String, String, String, String, String, String, String)> = stmt
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?, r.get(9)?, r.get(10)?))
            })
            .map(|it| it.filter_map(|x| x.ok()).collect())
            .unwrap_or_default();
        drop(stmt);
        let active: std::collections::HashSet<i64> = rows.iter().map(|r| r.0).collect();
        let title_of: std::collections::HashMap<i64, &str> =
            rows.iter().map(|r| (r.0, r.1.as_str())).collect();
        let now = chrono::Utc::now();
        let (mut ready, mut blocked) = (Vec::new(), Vec::new());
        // Hour resolution: a date-only review compares as its midnight.
        let today = now.format("%Y-%m-%dT%H:%MZ").to_string();
        for (id, title, rank, plan, note, blocked_by, updated, owner, plan_updated, kind, review_date) in &rows {
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
                // A lane carries its own deadline everywhere the CEO looks. It
                // was set from the brief on 2026-09-02 and then never shown
                // back here, so the date existed and did nothing.
                if review_date.is_empty() {
                    line.push_str(" — NO REVIEW DATE");
                } else if review_date.as_str() <= today.as_str() {
                    line.push_str(&format!(" — REVIEW DUE {review_date}: close it, drop it, or recommit with a new time"));
                } else if !horizon.is_empty() && review_date.as_str() > horizon {
                    line.push_str(&format!(" — review {review_date} is BEYOND THE HORIZON ({horizon}): a shelf, not a commitment — recommit inside it or close"));
                } else {
                    line.push_str(&format!(" — review {review_date}"));
                }
                if let Some(x) = extra.get(id) {
                    line.push_str(x);
                }
                // The mix is what "going in circles" looks like from the board:
                // an objective whose day was all checks of earlier work is not
                // advancing, however busy it reads.
                if let Some((built, checked)) = mix.get(id) {
                    line.push_str(&format!(" — 24h: {built} built / {checked} checks"));
                    if *built == 0 && *checked >= 3 {
                        line.push_str(" — ALL CHECKS, nothing built: CONVERT OR KILL");
                    }
                } else if kind == "explore" {
                    line.push_str(" — 24h: nothing dispatched");
                }
                if kind == "explore" && mix.get(id).is_none_or(|(b, _)| *b == 0) {
                    line.push_str(" — EXPLORE with no build in 24h: route a candidate to an execution lane or kill it");
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
            "SELECT id, title, kind, owner, note, plan FROM objectives WHERE status='active' ORDER BY rank, id",
        ) else {
            return String::new();
        };
        let objs: Vec<(i64, String, String, String, String, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))
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
        let line = |(id, title, kind, owner, note, plan): &(i64, String, String, String, String, String)| {
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
            // A money lane without a stated numeric premise can never be
            // falsified — every review then argues from vibes (a losing streak
            // in a low-hit-rate lane is near information-free; only actuals vs
            // a stated premise carry signal). The review nags until the plan
            // states the bet.
            if matches!(kind.as_str(), "profit" | "explore") && !plan.to_uppercase().contains("PREMISE") {
                l.push_str(
                    "\n  ⚠ no PREMISE line in its plan — state the numeric bet (expected return and cost; for lottery-shaped lanes the hit rate × payoff × trial-count budget) so this review can compare actuals against it instead of reacting to streaks.",
                );
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

    pub fn kv_del(&self, k: &str) {
        let c = self.conn.lock().unwrap();
        let _ = c.execute("DELETE FROM kv WHERE k=?1", params![k]);
    }

    pub fn kv_set(&self, k: &str, v: &str) {
        let c = self.conn.lock().unwrap();
        let _ = c.execute("INSERT INTO kv(k,v) VALUES(?1,?2) ON CONFLICT(k) DO UPDATE SET v=?2", params![k, v]);
    }

    /// Ticker events (stats, team): the site's stats daemon writes an ~80KB snapshot every
    /// 12 seconds and a team widget row beside it, using run_log as the bus
    /// to the viewer's event stream. That is 575MB a day into a 4.6GB volume;
    /// /data hit 100% on 2026-09-01 22:53Z and every routine died with ENOSPC.
    /// Only the latest few rows are ever read (page health looks at ten), so
    /// anything older than the window is dead weight.
    const TICKER_KEEP_SECS: i64 = 6 * 3600;

    /// Drop ticker rows older than the window. Called from log() on the path
    /// that keeps the table growing — the spill directory cleans itself the
    /// same way — so there is no cadence to schedule and nothing depends on a
    /// routine staying registered. Ceiling: freed pages are reused by SQLite
    /// but the file never shrinks without a VACUUM, which this deliberately
    /// does not run — it takes an exclusive lock and a temp copy the size of
    /// the database.
    pub fn prune_ticker(&self) -> usize {
        let cutoff = (chrono::Utc::now() - chrono::Duration::seconds(Self::TICKER_KEEP_SECS)).to_rfc3339();
        let c = self.conn.lock().unwrap();
        c.execute(
            "DELETE FROM run_log WHERE event IN ('stats','team') AND ts < ?1",
            params![cutoff],
        )
        .unwrap_or(0)
    }

    #[cfg(test)]
    pub fn raw_log_at(&self, ts: &str, agent: &str, event: &str, detail: &str) {
        let c = self.conn.lock().unwrap();
        let _ = c.execute(
            "INSERT INTO run_log(ts, agent, event, detail) VALUES(?1,?2,?3,?4)",
            params![ts, agent, event, detail],
        );
    }

    #[cfg(test)]
    pub fn log_events_for_test(&self) -> Vec<(String, String)> {
        let c = self.conn.lock().unwrap();
        let mut stmt = c.prepare("SELECT event, ts FROM run_log ORDER BY id").unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().flatten().collect()
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
        // Every 500th row: the binary logs several times a minute, so the
        // ticker's window is enforced within the hour regardless of what the
        // daemon does, and the delete costs nothing when there is nothing old.
        if id > 0 && id % 500 == 0 {
            self.prune_ticker();
        }
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

    pub fn record_model_call(&self, model: &str, ms: u64, ok: bool, err: &str, usage: crate::llm::Usage) {
        let c = self.conn.lock().unwrap();
        let _ = c.execute(
            "INSERT INTO model_calls(ts, model, ms, ok, err, prompt_tokens, completion_tokens, micros) \
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                chrono::Utc::now().to_rfc3339(),
                model,
                ms as i64,
                ok as i64,
                err,
                usage.prompt_tokens as i64,
                usage.completion_tokens as i64,
                usage.billed_micros as i64
            ],
        );
    }

    /// Calls answered against calls made over the last `hours`, once there are
    /// enough to mean something. A refusal counts as a failure: a seat that
    /// is under its speed floor or priced out of its cap is one nobody can be
    /// moved onto, however cheap its fills would be.
    pub fn success_rate(&self, model: &str, hours: i64) -> Option<(u64, u64)> {
        let since = (chrono::Utc::now() - chrono::Duration::hours(hours)).to_rfc3339();
        let c = self.conn.lock().unwrap();
        let (ok, n): (i64, i64) = c
            .query_row(
                "SELECT COALESCE(SUM(ok),0), COUNT(*) FROM model_calls WHERE model=?1 AND ts>?2",
                params![model, since],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok()?;
        (n >= 5).then_some((ok as u64, n as u64))
    }

    /// What a model has actually cost this company lately: settled
    /// micro-dollars per million tokens (prompt and completion together, so
    /// the blend is our real mix, not a guess), over the last `hours`. None
    /// until there are enough fills to mean something.
    ///
    /// Deliberate ceiling: fills at the gateway's minimum charge (~$0.002 for
    /// settlement gas) are left out, since a 20-token answer billed at the
    /// floor says nothing about the per-token rate; if every call is that
    /// small the price is unknown rather than wrong.
    pub fn realized_price(&self, model: &str, hours: i64) -> Option<u64> {
        let since = (chrono::Utc::now() - chrono::Duration::hours(hours)).to_rfc3339();
        let c = self.conn.lock().unwrap();
        let (n, micros, tokens): (i64, i64, i64) = c
            .query_row(
                "SELECT COUNT(*), COALESCE(SUM(micros),0), COALESCE(SUM(prompt_tokens+completion_tokens),0) \
                 FROM model_calls WHERE model=?1 AND ts>?2 AND ok=1 AND micros>2000 AND prompt_tokens+completion_tokens>0",
                params![model, since],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .ok()?;
        (n >= 5 && tokens > 0).then(|| (micros as u128 * 1_000_000 / tokens as u128) as u64)
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

    pub fn upsert_routine(&self, name: &str, command: &str, interval_secs: i64, purpose: &str, owner: &str) {
        let c = self.conn.lock().unwrap();
        let _ = c.execute(
            "INSERT INTO routines(name, command, interval_secs, purpose, enabled, last_run, owner)
             VALUES(?1,?2,?3,?4,1,0,?5)
             ON CONFLICT(name) DO UPDATE SET command=?2, interval_secs=?3, purpose=?4, enabled=1, owner=?5",
            params![name, command, interval_secs, purpose, owner],
        );
    }

    /// Assign (or clear, with "") the owning agent of an existing routine.
    /// Returns false when no such routine exists.
    pub fn set_routine_owner(&self, name: &str, owner: &str) -> bool {
        let c = self.conn.lock().unwrap();
        c.execute("UPDATE routines SET owner=?2 WHERE name=?1", params![name, owner])
            .map(|n| n > 0)
            .unwrap_or(false)
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
                    interval_secs, COALESCE(purpose,''),
                    COALESCE(last_status,'never ran')
                      || CASE WHEN COALESCE(agent,'') != '' THEN ''
                              WHEN COALESCE(owner,'') != '' THEN ' — alerts owned by ' || owner
                              ELSE ' — alerts wake the CEO (assign an owner with own_routine)' END
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
    pub fn due_routines(&self, now_epoch: i64) -> Vec<(String, String, String, String, String)> {
        let c = self.conn.lock().unwrap();
        let mut stmt = match c.prepare(
            "SELECT name, command, COALESCE(agent,''), COALESCE(task,''), COALESCE(owner,'') FROM routines
             WHERE enabled=1 AND (last_run = 0 OR ?1 - last_run >= interval_secs) ORDER BY name",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map(params![now_epoch], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
            .map(|it| it.filter_map(|x| x.ok()).collect())
            .unwrap_or_default()
    }

    /// Shell routines owned by `owner`: how many report ok, how many exist,
    /// and the names of the ones that do not — the status line of an ops lane.
    pub fn routine_status_for_owner(&self, owner: &str) -> (usize, usize, Vec<String>) {
        let c = self.conn.lock().unwrap();
        let Ok(mut stmt) = c.prepare(
            "SELECT name, COALESCE(last_status,'never ran') FROM routines \
             WHERE enabled=1 AND COALESCE(agent,'')='' AND owner=?1 ORDER BY name",
        ) else {
            return (0, 0, Vec::new());
        };
        let rows: Vec<(String, String)> = stmt
            .query_map(params![owner], |r| Ok((r.get(0)?, r.get(1)?)))
            .map(|it| it.filter_map(|x| x.ok()).collect())
            .unwrap_or_default();
        let failing: Vec<String> = rows.iter().filter(|(_, s)| s != "ok").map(|(n, s)| format!("{n} ({s})")).collect();
        (rows.len() - failing.len(), rows.len(), failing)
    }

    /// Make every shell routine due now. On boot this is the restart triage:
    /// the scripts re-establish page health, ledger match and the rest within
    /// a minute, at zero model cost, instead of the CEO re-deriving them by
    /// hand (twenty minutes and a dozen shells after the 2026-09-03 restart).
    /// Review routines are left on their clocks — they dispatch a model.
    pub fn reset_routine_clocks(&self) -> usize {
        let c = self.conn.lock().unwrap();
        c.execute("UPDATE routines SET last_run=0 WHERE enabled=1 AND COALESCE(agent,'')=''", [])
            .unwrap_or(0)
    }

    /// Active objectives as (id, kind, owner), for the binary-side board extras.
    pub fn active_objective_meta(&self) -> Vec<(i64, String, String)> {
        let c = self.conn.lock().unwrap();
        let Ok(mut stmt) = c.prepare("SELECT id, COALESCE(kind,''), COALESCE(owner,'') FROM objectives WHERE status='active'") else {
            return Vec::new();
        };
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map(|it| it.filter_map(|x| x.ok()).collect())
            .unwrap_or_default()
    }

    /// The most recent report an agent filed, if any.
    pub fn last_report(&self, agent: &str) -> Option<String> {
        let c = self.conn.lock().unwrap();
        c.query_row(
            "SELECT COALESCE(detail,'') FROM run_log WHERE agent=?1 AND event='report' ORDER BY id DESC LIMIT 1",
            params![agent],
            |r| r.get(0),
        )
        .ok()
    }

    /// An agent's most recent dispatch: (objective, class).
    pub fn latest_dispatch(&self, agent: &str) -> Option<(Option<i64>, String)> {
        let c = self.conn.lock().unwrap();
        c.query_row(
            "SELECT objective, class FROM dispatches WHERE agent=?1 ORDER BY id DESC LIMIT 1",
            params![agent],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok()
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

    /// The X budget ledger: the ONLY balance the company ever sees (there is no
    /// balance endpoint on the pay-per-use plan, and agents must never guess
    /// from the X console). Seeded with the founder's $5; every API call
    /// debits, on-chain-verified USDC top-ups credit.
    pub fn x_balance(&self) -> f64 {
        let c = self.conn.lock().unwrap();
        c.query_row(
            "SELECT COALESCE(SUM(CASE WHEN kind='topup' THEN amount_usd ELSE -amount_usd END), 0) FROM x_ledger",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0.0)
    }

    /// Record a spend and return the new balance. The first debit that takes
    /// the balance under $1 raises one routine alert (kv-flagged so a low
    /// balance nags once, not on every call; a top-up re-arms it).
    pub fn x_debit(&self, amount_usd: f64, detail: &str) -> f64 {
        {
            let c = self.conn.lock().unwrap();
            let _ = c.execute(
                "INSERT INTO x_ledger(ts, kind, amount_usd, detail) VALUES(datetime('now'),'spend',?1,?2)",
                params![amount_usd, detail],
            );
        }
        let bal = self.x_balance();
        if bal < 1.0 && self.kv_get("x_low_alerted").is_none() {
            self.kv_set("x_low_alerted", "1");
            self.add_routine_alert(
                "x-budget",
                &format!("X budget is down to ${bal:.3}. Paid X calls refuse at $0 — top up BEFORE it runs out: send USDC on Solana to the fund address (x_read mode budget shows it), then record the tx with x_topup."),
            );
        }
        bal
    }

    /// Record which X resources were returned today and report how many are
    /// NEW. X bills per distinct resource per UTC day (24h deduplication,
    /// docs.x.com pricing, verified 2026-09-01): a post fetched twice in one
    /// day is charged once, so the ledger must only debit first sightings or
    /// it drifts pessimistic and strands real prepaid credits at a fake $0.
    /// `day` is the caller's UTC date; rows from other days are purged here,
    /// keeping the table one day wide.
    // --- who we have already answered ---
    //
    // x_seen above is a BILLING ledger: X charges once per tweet per UTC day,
    // so it is wiped at midnight. It was doing double duty as the "have we
    // handled this?" memory, and at midnight that memory emptied — tweets
    // 2094781874174357640 and 2094913224747737227 each got a second reply the
    // next day. A reply is permanent, so its record is too.

    /// (when, our reply's tweet id) if we have already replied to this tweet.
    pub fn x_reply_to(&self, tweet_id: &str) -> Option<(String, String)> {
        let c = self.conn.lock().unwrap();
        c.query_row(
            "SELECT ts, our_id FROM x_replied WHERE tweet_id=?1",
            params![tweet_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok()
    }

    pub fn x_record_reply(&self, tweet_id: &str, our_id: &str) {
        let c = self.conn.lock().unwrap();
        let _ = c.execute(
            "INSERT OR IGNORE INTO x_replied(tweet_id, ts, our_id) VALUES(?1,?2,?3)",
            params![tweet_id, chrono::Utc::now().to_rfc3339(), our_id],
        );
    }

    pub fn x_mark_seen(&self, ids: &[&str], day: &str) -> usize {
        let c = self.conn.lock().unwrap();
        let _ = c.execute("DELETE FROM x_seen WHERE day <> ?1", params![day]);
        let mut new = 0;
        for rid in ids {
            if c.execute(
                "INSERT OR IGNORE INTO x_seen(rid, day) VALUES(?1, ?2)",
                params![rid, day],
            )
            .unwrap_or(0)
                > 0
            {
                new += 1;
            }
        }
        new
    }

    /// Credit a verified top-up and return the new balance. Re-arms the
    /// low-balance alert.
    pub fn x_topup_credit(&self, amount_usd: f64, detail: &str) -> f64 {
        {
            let c = self.conn.lock().unwrap();
            let _ = c.execute(
                "INSERT INTO x_ledger(ts, kind, amount_usd, detail) VALUES(datetime('now'),'topup',?1,?2)",
                params![amount_usd, detail],
            );
        }
        self.kv_del("x_low_alerted");
        self.x_balance()
    }

    /// True if a ledger row already references this detail substring — the
    /// dedup that stops one Solana tx from being credited twice.
    pub fn x_ledger_has(&self, needle: &str) -> bool {
        let c = self.conn.lock().unwrap();
        c.query_row(
            "SELECT COUNT(*) FROM x_ledger WHERE detail LIKE '%' || ?1 || '%'",
            params![needle],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| n > 0)
        .unwrap_or(false)
    }

    /// Newest ledger rows, one line each, for the budget view.
    pub fn x_ledger_tail(&self, n: i64) -> Vec<String> {
        let c = self.conn.lock().unwrap();
        let mut stmt = match c.prepare(
            "SELECT ts, kind, amount_usd, detail FROM x_ledger ORDER BY id DESC LIMIT ?1",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map(params![n], |r| {
            let (ts, kind, amt, detail): (String, String, f64, String) =
                (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?);
            let sign = if kind == "topup" { "+" } else { "-" };
            Ok(format!("[{ts}] {sign}${amt:.3} {detail}"))
        })
        .map(|it| it.filter_map(|x| x.ok()).collect())
        .unwrap_or_default()
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

    /// Undelivered founder messages with their ids, oldest first; marks them
    /// delivered. Delivery is not acknowledgement — see open_directives.
    pub fn drain_messages(&self) -> Vec<(i64, String)> {
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
        rows
    }

    /// `khan tell` messages delivered to the CEO and not yet acknowledged,
    /// oldest first: (id, created_at, msg). Telegram turns are conversation,
    /// not standing directives, and are excluded — they carry their own
    /// context through the telegram brief.
    pub fn open_directives(&self) -> Vec<(i64, String, String)> {
        let c = self.conn.lock().unwrap();
        let mut stmt = match c.prepare(
            "SELECT id, created_at, msg FROM messages WHERE delivered=1 AND acked=0 AND msg NOT LIKE '[via Telegram]%' ORDER BY id",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map(|it| it.filter_map(|x| x.ok()).collect())
            .unwrap_or_default()
    }

    /// Mark a founder directive done. False when no such open directive.
    pub fn ack_message(&self, id: i64) -> bool {
        let c = self.conn.lock().unwrap();
        c.execute("UPDATE messages SET acked=1 WHERE id=?1 AND delivered=1 AND acked=0", params![id])
            .map(|n| n > 0)
            .unwrap_or(false)
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

    /// The newest version of a prompt that is actually a prompt. On
    /// 2026-08-29 the CEO saved version 11 of its own prompt as a bare URL to
    /// a file that does not exist, and ran for five days on the mandate alone
    /// — the mission, the flagship, the risk rules all gone from context, with
    /// nothing anywhere saying so. A version that fails `prompt_usable` is
    /// skipped, so the last real one stays live.
    pub fn get_prompt(&self, name: &str) -> Option<String> {
        let c = self.conn.lock().unwrap();
        let Ok(mut stmt) = c.prepare("SELECT content FROM prompts WHERE name=?1 ORDER BY version DESC") else {
            return None;
        };
        let versions: Vec<String> = stmt
            .query_map(params![name], |r| r.get::<_, String>(0))
            .map(|it| it.flatten().collect())
            .unwrap_or_default();
        versions.into_iter().find(|p| prompt_usable(p))
    }

    pub fn update_prompt(&self, name: &str, content: &str, reason: &str) -> Result<i64> {
        if !prompt_usable(content) {
            anyhow::bail!(
                "a prompt is the full text an agent runs on, not a pointer: this is {} characters{} — pass the whole prompt (edit the current one and send it back complete)",
                content.trim().chars().count(),
                if content.trim().starts_with("http") { " and reads as a URL, which nothing fetches" } else { "" }
            );
        }
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

    /// The skills worth indexing on every call: loaded within `loaded_days`
    /// or created within `created_days`. 243 skills rode every request of
    /// every agent on 2026-09-02 (19k characters, roughly 5k tokens, 11,800
    /// times a day) while 60 of them had been loaded in the past week.
    pub fn recent_skills(&self, loaded_days: i64, created_days: i64) -> Vec<(String, String)> {
        let loaded = (chrono::Utc::now() - chrono::Duration::days(loaded_days)).to_rfc3339();
        let created = (chrono::Utc::now() - chrono::Duration::days(created_days)).to_rfc3339();
        let c = self.conn.lock().unwrap();
        let Ok(mut stmt) = c.prepare(
            "SELECT name, description FROM skill_defs s
             WHERE version=(SELECT MAX(version) FROM skill_defs WHERE name=s.name)
               AND (name IN (SELECT skill FROM skill_loads WHERE ts>?1) OR created_at>?2)
             ORDER BY name",
        ) else {
            return vec![];
        };
        stmt.query_map(params![loaded, created], |r| Ok((r.get(0)?, r.get(1)?)))
            .map(|it| it.filter_map(|x| x.ok()).collect())
            .unwrap_or_default()
    }

    /// Skills whose name or description contains `q` (case-insensitive).
    pub fn search_skills(&self, q: &str) -> Vec<(String, String)> {
        let needle = q.to_lowercase();
        self.list_skills()
            .into_iter()
            .filter(|(n, d)| n.to_lowercase().contains(&needle) || d.to_lowercase().contains(&needle))
            .collect()
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
