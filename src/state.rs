use anyhow::Result;
use rusqlite::{params, Connection};
use std::sync::Mutex;
use tokio::sync::broadcast;

fn log_row_json(id: i64, ts: &str, agent: &str, event: &str, detail: &str) -> String {
    serde_json::json!({"id": id, "ts": ts, "agent": agent, "event": event, "detail": detail}).to_string()
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
             CREATE TABLE IF NOT EXISTS ratings (
                id INTEGER PRIMARY KEY AUTOINCREMENT, agent TEXT NOT NULL, score INTEGER NOT NULL,
                note TEXT, prompt_version INTEGER, created_at TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS run_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT NOT NULL,
                agent TEXT NOT NULL, event TEXT NOT NULL, detail TEXT);
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
        Ok(Store { conn: Mutex::new(conn), log_tx: broadcast::channel(512).0 })
    }

    pub fn subscribe_log(&self) -> broadcast::Receiver<String> {
        self.log_tx.subscribe()
    }

    pub fn kv_get(&self, k: &str) -> Option<String> {
        let c = self.conn.lock().unwrap();
        c.query_row("SELECT v FROM kv WHERE k=?1", params![k], |r| r.get(0)).ok()
    }

    pub fn kv_set(&self, k: &str, v: &str) {
        let c = self.conn.lock().unwrap();
        let _ = c.execute("INSERT INTO kv(k,v) VALUES(?1,?2) ON CONFLICT(k) DO UPDATE SET v=?2", params![k, v]);
    }

    pub fn log(&self, agent: &str, event: &str, detail: &str) {
        let ts = chrono::Utc::now().to_rfc3339();
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
        let _ = self.log_tx.send(log_row_json(id, &ts, agent, event, detail));
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
        let mut stmt = match c.prepare(
            "SELECT ts, agent, event, detail FROM run_log ORDER BY id DESC LIMIT ?1",
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

    // --- founder messages (khan tell) ---

    pub fn queue_message(&self, msg: &str) {
        let c = self.conn.lock().unwrap();
        let _ = c.execute(
            "INSERT INTO messages(msg, created_at) VALUES(?1,?2)",
            params![msg, chrono::Utc::now().to_rfc3339()],
        );
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
        let _ = c.execute(
            "INSERT INTO ratings(agent, score, note, prompt_version, created_at) VALUES(?1,?2,?3,?4,?5)",
            params![agent, score, note, pv, chrono::Utc::now().to_rfc3339()],
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
        stmt.query_map(params![fts, limit], |r| {
            let (k, v): (String, String) = (r.get(0)?, r.get(1)?);
            Ok(format!("[{k}] {v}"))
        })
        .map(|it| it.filter_map(|x| x.ok()).collect())
        .unwrap_or_default()
    }
}
