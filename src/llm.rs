use crate::config::Config;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// The model's own reasoning, when it separates that from its answer.
    /// Providers disagree on the name, hence the alias. Never serialized: it is
    /// for display only, and sending it back can be rejected upstream.
    #[serde(default, alias = "reasoning_content", skip_serializing)]
    pub reasoning: Option<String>,
}

impl Message {
    pub fn text(role: &str, content: impl Into<String>) -> Message {
        Message { role: role.into(), content: Some(content.into()), tool_calls: None, tool_call_id: None, reasoning: None }
    }
    pub fn tool_result(id: &str, content: impl Into<String>) -> Message {
        Message { role: "tool".into(), content: Some(content.into()), tool_calls: None, tool_call_id: Some(id.into()), reasoning: None }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type", default = "func_type")]
    pub kind: String,
    pub function: FunctionCall,
}

fn func_type() -> String {
    "function".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

/// The model spent its entire output budget before it produced anything: HTTP
/// 200, empty content, `finish_reason` "length".
///
/// Carried as its own error type because the cure is specific. Retrying against
/// another model cannot help — the request is identical, so the next model spends
/// the same budget the same way — and failing the task outright throws away work
/// over what is really a task-shaping problem. The caller asks for a smaller step.
#[derive(Debug, Clone, Copy)]
pub struct Truncated {
    pub max_tokens: u32,
    pub reasoning_tokens: u64,
}

impl std::fmt::Display for Truncated {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "used its entire {}-token output budget on reasoning ({} reasoning tokens) \
and never reached an answer (finish_reason=length)",
            self.max_tokens, self.reasoning_tokens
        )
    }
}

impl std::error::Error for Truncated {}

/// The truncation behind an error, when that is what it was.
pub fn truncation(e: &anyhow::Error) -> Option<Truncated> {
    e.downcast_ref::<Truncated>().copied()
}

/// Request caps that apply to a provider's free model variants.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub rpm: u32,
    pub rpd: u32,
    /// True when the account has never purchased credits — the low daily cap.
    pub is_free_tier: bool,
}

/// OpenRouter's published free-variant caps.
/// Source: https://openrouter.ai/docs/api-reference/limits
const OR_FREE_RPM: u32 = 20;
const OR_FREE_RPD_UNPAID: u32 = 50;
const OR_FREE_RPD_PAID: u32 = 1000;

#[derive(Default)]
struct FreeUsage {
    minute_start: Option<Instant>,
    minute_count: u32,
    day: String,
    day_count: u32,
}

pub struct Client {
    http: reqwest::Client,
    /// Model id -> the moment it becomes usable again. Checked before every
    /// request, so a rate-limited model is skipped instead of hit again.
    cooldown: Mutex<HashMap<String, Instant>>,
    /// Locally counted use of `:free` variants, which share one account-wide cap.
    free: Mutex<FreeUsage>,
    limits: Mutex<Option<Limits>>,
    /// "provider/model" -> context window in tokens, for providers that publish it.
    /// Absent means unknown, which callers must treat as "no opinion" rather than
    /// as a small window.
    ctx_limits: Mutex<HashMap<String, u32>>,
    /// "provider/model" -> most output tokens the provider will accept for one
    /// answer. Published separately from the window and often far smaller than it:
    /// a model with a 1M window can still cap a single answer at 16k.
    out_limits: Mutex<HashMap<String, u32>>,
}

/// Never ask for more output than this, however much a model advertises.
///
/// This value is also what compaction reserves out of the context window (see
/// `agent::compact_threshold`), so honouring a model that advertises 512k would
/// squeeze the history down to nothing. 64k is far more than any single answer
/// needs and still four times the old fixed ceiling.
const MAX_OUTPUT: u32 = 65_536;

impl Client {
    pub fn new() -> Client {
        Client {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(300))
                .build()
                .expect("http client"),
            cooldown: Mutex::new(HashMap::new()),
            free: Mutex::new(FreeUsage::default()),
            limits: Mutex::new(None),
            ctx_limits: Mutex::new(HashMap::new()),
            out_limits: Mutex::new(HashMap::new()),
        }
    }

    /// Learn each model's context window where the provider publishes one.
    ///
    /// Only OpenRouter does: its catalog carries `context_length`, and it rejects a
    /// request outright when prompt + max_tokens exceeds it. bu0y's catalog is
    /// prices only, so its models stay unknown here — which is correct, because it
    /// clamps an oversized ceiling instead of failing.
    ///
    /// Best effort by design. A failure leaves the map empty, and an empty map
    /// means every caller keeps its own default.
    pub async fn discover_context_limits(&self, cfg: &Config) {
        let Some(prov) = cfg.providers.iter().find(|p| p.base_url.contains("openrouter.ai")) else {
            return;
        };
        let url = format!("{}/models", prov.base_url.trim_end_matches('/'));
        let Ok(resp) = self.http.get(&url).send().await else { return };
        let Ok(v) = resp.json::<Value>().await else { return };
        let Some(list) = v["data"].as_array() else { return };
        let mut ctxs = self.ctx_limits.lock().unwrap();
        let mut outs = self.out_limits.lock().unwrap();
        for m in list {
            let Some(id) = m["id"].as_str() else { continue };
            let key = format!("{}/{id}", prov.name);
            if let Some(ctx) = m["context_length"].as_u64().filter(|c| *c > 0) {
                ctxs.insert(key.clone(), ctx.min(u32::MAX as u64) as u32);
            }
            // Recorded independently of the window: the two are unrelated in
            // practice, and a model missing one may still publish the other.
            if let Some(out) = m["top_provider"]["max_completion_tokens"].as_u64().filter(|c| *c > 0)
            {
                outs.insert(key, out.min(u32::MAX as u64) as u32);
            }
        }
    }

    /// Test-only: seed a published ceiling without a network round-trip.
    #[cfg(test)]
    pub fn set_output_limit(&self, model: &str, limit: u32) {
        self.out_limits.lock().unwrap().insert(model.to_string(), limit);
    }

    /// Output ceiling to send for a model: what the provider says it accepts,
    /// bounded by `MAX_OUTPUT`.
    ///
    /// A provider that publishes nothing (bu0y lists prices only) keeps the
    /// configured default, which is right for it — it clamps an oversized ceiling
    /// rather than rejecting the request.
    pub fn output_limit(&self, model: &str, cfg: &Config) -> u32 {
        self.out_limits
            .lock()
            .unwrap()
            .get(model)
            .map(|&c| c.min(MAX_OUTPUT))
            .unwrap_or(cfg.max_tokens)
    }

    /// Context window for a "provider/model", if the provider published one.
    pub fn context_limit(&self, model: &str) -> Option<u32> {
        self.ctx_limits.lock().unwrap().get(model).copied()
    }

    /// Ask OpenRouter what caps this key actually gets rather than assuming.
    /// `is_free_tier` goes false once the account has ever purchased credits, and
    /// that is what raises the daily cap from 50 to 1000. Best effort: if the call
    /// fails, khan keeps the pessimistic cap and under-uses rather than gets blocked.
    pub async fn discover_limits(&self, cfg: &Config) -> Option<Limits> {
        let prov = cfg.providers.iter().find(|p| p.base_url.contains("openrouter.ai"))?;
        let key = cfg.key_for(&prov.name)?;
        let url = format!("{}/key", prov.base_url.trim_end_matches('/'));
        let v: Value =
            self.http.get(&url).bearer_auth(key).send().await.ok()?.json().await.ok()?;
        let is_free_tier = v["data"]["is_free_tier"].as_bool().unwrap_or(true);
        let lim = Limits {
            rpm: OR_FREE_RPM,
            rpd: if is_free_tier { OR_FREE_RPD_UNPAID } else { OR_FREE_RPD_PAID },
            is_free_tier,
        };
        *self.limits.lock().unwrap() = Some(lim);
        Some(lim)
    }

    /// Remaining cooldown for a model, if it is currently rate limited.
    fn cooling(&self, model: &str) -> Option<Duration> {
        let mut c = self.cooldown.lock().unwrap();
        let now = Instant::now();
        match c.get(model) {
            Some(&until) if until > now => Some(until - now),
            Some(_) => {
                c.remove(model);
                None
            }
            None => None,
        }
    }

    fn set_cooldown(&self, model: &str, d: Duration) {
        self.cooldown.lock().unwrap().insert(model.to_string(), Instant::now() + d);
    }

    /// Count a `:free` request against the account-wide caps. Returns how long to
    /// wait if a cap is already reached, so khan stops before the 429 rather than
    /// discovering it by spending requests it does not have.
    fn charge_free_quota(&self, model_id: &str) -> Option<Duration> {
        if !model_id.ends_with(":free") {
            return None;
        }
        let lim = (*self.limits.lock().unwrap())?;
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let mut u = self.free.lock().unwrap();
        if u.day != today {
            u.day = today;
            u.day_count = 0;
        }
        if u.day_count >= lim.rpd {
            // Daily caps reset at UTC midnight — wait it out instead of retrying.
            let secs = 86_400 - chrono::Utc::now().timestamp().rem_euclid(86_400) + 5;
            return Some(Duration::from_secs(secs as u64));
        }
        if u.minute_start.map(|s| s.elapsed() >= Duration::from_secs(60)).unwrap_or(true) {
            u.minute_start = Some(Instant::now());
            u.minute_count = 0;
        }
        if u.minute_count >= lim.rpm {
            let waited = u.minute_start.map(|s| s.elapsed()).unwrap_or_default();
            return Some(Duration::from_secs(61).saturating_sub(waited));
        }
        u.minute_count += 1;
        u.day_count += 1;
        None
    }

    pub fn build_request(model_id: &str, messages: &[Message], tools: &[Value], max_tokens: u32) -> Value {
        // max_tokens is always sent. Omitting it lets the gateway impose its own
        // small ceiling, and a reasoning model will burn the lot thinking and
        // return an empty answer.
        let mut body = serde_json::json!({
            "model": model_id, "messages": messages, "max_tokens": max_tokens
        });
        if !tools.is_empty() {
            body["tools"] = Value::Array(tools.to_vec());
        }
        body
    }

    /// Chat completion against any OpenAI-compatible endpoint. `model` is "provider/model".
    pub async fn chat(
        &self,
        cfg: &Config,
        model: &str,
        messages: &[Message],
        tools: &[Value],
    ) -> Result<(Message, Usage)> {
        let (prov, model_id, key) = cfg.resolve(model)?;
        if let Some(left) = self.cooling(model) {
            bail!("{model} is rate limited for another {}s - not retried", left.as_secs());
        }
        if let Some(wait) = self.charge_free_quota(&model_id) {
            // A cap we can see locally: don't spend a request confirming it.
            self.set_cooldown(model, wait);
            bail!("{model} hit its free-tier request cap; paused for {}s", wait.as_secs());
        }
        let url = format!("{}/chat/completions", prov.base_url.trim_end_matches('/'));
        let max_out = self.output_limit(model, cfg);
        let body = Self::build_request(&model_id, messages, tools, max_out);

        let mut last_err = String::new();
        for attempt in 0..4u32 {
            if attempt > 0 {
                // Announce every retry. Each attempt can block for the full 300s
                // timeout, so without this the loop sits silent for up to ~20
                // minutes and a slow provider is indistinguishable from a hang.
                eprintln!("llm: {model} attempt {}/4 failed, retrying — {last_err}", attempt);
                tokio::time::sleep(Duration::from_secs(2u64.pow(attempt))).await;
            }
            // App attribution (used by OpenRouter's dashboard; harmless elsewhere).
            let resp = self
                .http
                .post(&url)
                .bearer_auth(&key)
                .header("X-Title", "khan")
                .header("HTTP-Referer", "https://khan.local")
                .json(&body)
                .send()
                .await;
            let resp = match resp {
                Ok(r) => r,
                Err(e) => {
                    last_err = format!("request error: {e}");
                    continue;
                }
            };
            let status = resp.status();
            let reset = retry_after(resp.headers());
            let text = resp.text().await.unwrap_or_default();
            if status.as_u16() == 429 {
                // A rate limit does not clear in a few seconds of backoff, and each
                // extra attempt counts against the very cap that was just hit. Park
                // the model until the reset the server named, and stop retrying.
                let d = reset.unwrap_or(Duration::from_secs(60));
                self.set_cooldown(model, d);
                bail!(
                    "{model} rate limited (429), paused for {}s: {}",
                    d.as_secs(),
                    text.chars().take(200).collect::<String>()
                );
            }
            if status.is_server_error() {
                last_err = format!("{status}: {}", text.chars().take(300).collect::<String>());
                continue;
            }
            if !status.is_success() {
                bail!("{model} returned {status}: {}", text.chars().take(500).collect::<String>());
            }
            let v: Value = serde_json::from_str(&text).context("invalid JSON from provider")?;
            let msg_v = v["choices"][0]["message"].clone();
            if msg_v.is_null() {
                bail!("no message in response from {model}: {}", text.chars().take(500).collect::<String>());
            }
            let msg: Message = serde_json::from_value(msg_v).context("unexpected message shape")?;
            let usage = Usage {
                prompt_tokens: v["usage"]["prompt_tokens"].as_u64().unwrap_or(0),
                completion_tokens: v["usage"]["completion_tokens"].as_u64().unwrap_or(0),
            };
            // A reasoning model can spend its whole output budget thinking and stop
            // before it answers: HTTP 200, empty content, finish_reason "length".
            // Providers say so plainly; not reading it is what made these look like
            // silent empty replies.
            let finish = v["choices"][0]["finish_reason"].as_str().unwrap_or("");
            let nothing = msg.content.as_deref().is_none_or(|c| c.trim().is_empty())
                && msg.tool_calls.as_ref().is_none_or(|t| t.is_empty());
            if nothing && finish == "length" {
                let think = v["usage"]["completion_tokens_details"]["reasoning_tokens"]
                    .as_u64()
                    .unwrap_or(0);
                return Err(anyhow::Error::new(Truncated {
                    max_tokens: max_out,
                    reasoning_tokens: think,
                })
                .context(model.to_string()));
            }
            return Ok((msg, usage));
        }
        bail!("{model} failed after retries: {last_err}")
    }
}

/// Reset delay advertised by a 429. OpenRouter sends `X-RateLimit-Reset` on its own
/// platform limits, and `Retry-After` when every attempted provider gave a hint.
pub(crate) fn retry_after(h: &reqwest::header::HeaderMap) -> Option<Duration> {
    fn num(v: &reqwest::header::HeaderValue) -> Option<i64> {
        v.to_str().ok()?.trim().parse::<i64>().ok()
    }
    if let Some(n) = h.get("retry-after").and_then(num) {
        if n > 0 {
            return Some(Duration::from_secs(n.min(86_400) as u64));
        }
    }
    let n = h.get("x-ratelimit-reset").and_then(num)?;
    let now_ms = chrono::Utc::now().timestamp_millis();
    // The value may be an epoch in ms, an epoch in seconds, or a plain delta.
    let delta = if n > 1_000_000_000_000 {
        n - now_ms
    } else if n > 1_000_000_000 {
        n * 1000 - now_ms
    } else {
        n * 1000
    };
    (delta > 0).then(|| Duration::from_millis(delta.min(86_400_000) as u64))
}
