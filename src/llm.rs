use crate::config::Config;
use anyhow::{bail, Result};
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
    /// Images shown to the model with this message, as data: URLs. Kept out of
    /// `content` so every reader of the string (compaction, length accounting,
    /// the public log) keeps working; build_request expands them into
    /// OpenAI content parts at send time. Never serialized: they live only in
    /// the in-memory history — a reloaded history re-reads the file if it
    /// needs the picture again.
    #[serde(default, skip_serializing)]
    pub images: Option<Vec<String>>,
}

impl Message {
    pub fn text(role: &str, content: impl Into<String>) -> Message {
        Message { role: role.into(), content: Some(content.into()), tool_calls: None, tool_call_id: None, reasoning: None, images: None }
    }
    pub fn tool_result(id: &str, content: impl Into<String>) -> Message {
        Message { role: "tool".into(), content: Some(content.into()), tool_calls: None, tool_call_id: Some(id.into()), reasoning: None, images: None }
    }
    /// A user turn carrying pictures. Tool messages cannot hold images under
    /// the OpenAI shape, so an image-bearing tool result is followed by one of
    /// these.
    pub fn with_images(role: &str, content: impl Into<String>, images: Vec<String>) -> Message {
        Message { role: role.into(), content: Some(content.into()), tool_calls: None, tool_call_id: None, reasoning: None, images: Some(images) }
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
    /// True when the ceiling was the gateway's `retry_max_tokens`, not ours.
    /// That number is derived from the model's RECENT SPEED, so a degraded
    /// route hands back a ceiling too small to answer inside — and unlike a
    /// budget we chose, another model would be given a different one. The
    /// caller uses this to decide whether walking the ladder can help.
    pub gateway_capped: bool,
}

impl std::fmt::Display for Truncated {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "used its entire {}-token output budget{} on reasoning ({} reasoning tokens) \
and never reached an answer (finish_reason=length)",
            self.max_tokens,
            if self.gateway_capped { " (the ceiling the gateway said would fit)" } else { "" },
            self.reasoning_tokens
        )
    }
}

impl std::error::Error for Truncated {}

/// True when a gateway 5xx carries an upstream read timeout rather than a
/// transient fault.
///
/// The gateway surfaces these as a clean 502 whose body names the real upstream
/// status, because it deliberately does not retry them itself: the origin accepted
/// the request and kept generating, so the work is done and may be billed whether
/// or not anyone reads it. Retrying doubles the bill for one answer.
pub(crate) fn upstream_timeout(body: &str) -> bool {
    body.contains("upstream status 524") || body.contains("upstream status 504")
}

/// `error.retry_max_tokens` from a refusal: the largest output ceiling the
/// gateway says would actually fill.
///
/// It rides a 503 when no offer clears the floor, and now also a 400 when the
/// ask is larger than the model can produce inside the fill ceiling at its
/// recent speed. That 400 is a retry, not a bad request — abandoning it drops
/// the call for a number the gateway already handed us.
pub(crate) fn retry_max_tokens(body: &str) -> Option<u32> {
    serde_json::from_str::<Value>(body)
        .ok()?
        .get("error")?
        .get("retry_max_tokens")?
        .as_u64()
        .filter(|n| *n > 0)
        .map(|n| n.min(MAX_OUTPUT as u64) as u32)
}

/// A 503 whose type is `unmet_speed`: every route is slower than the floor we
/// sent. Type-checked rather than text-matched because the message carries a
/// number that changes.
pub(crate) fn unmet_speed(body: &str) -> bool {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v["error"]["type"].as_str().map(|t| t == "unmet_speed"))
        .unwrap_or(false)
}

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
                // Above the gateway's absolute fill ceiling, which moved from
                // 300s to 480s. A client that hangs up first is billed for
                // everything already delivered and gets none of it.
                .timeout(Duration::from_secs(540))
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
        for prov in &cfg.providers {
            self.discover_from(cfg, prov).await;
        }
    }

    /// Read one provider's catalog. Best effort per provider, so a gateway that
    /// publishes nothing — or is simply down — leaves the others intact.
    async fn discover_from(&self, cfg: &Config, prov: &crate::config::Provider) {
        let url = format!("{}/models", prov.base_url.trim_end_matches('/'));
        let mut req = self.http.get(&url);
        // OpenRouter serves this publicly; other gateways want the key. Sending it
        // to the provider it belongs to is safe and costs nothing when unused.
        if let Some(key) = cfg.key_for(&prov.name) {
            req = req.bearer_auth(key);
        }
        let Ok(resp) = req.send().await else { return };
        let Ok(v) = resp.json::<Value>().await else { return };
        // Some gateways return a bare array rather than the OpenAI {"data": [...]}.
        let Some(list) = v["data"].as_array().or_else(|| v.as_array()) else { return };
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

    /// Seats that reject image content parts. glm53flash, glm53, glm5 and
    /// grok46 all take vision; the fallback rungs below do not.
    pub fn text_only_model(model_id: &str) -> bool {
        let m = model_id.to_ascii_lowercase();
        ["deepseek", "kimi", "minimax"].iter().any(|t| m.contains(t))
    }

    pub fn build_request(model_id: &str, messages: &[Message], tools: &[Value], max_tokens: u32) -> Value {
        // max_tokens is always sent. Omitting it lets the gateway impose its own
        // small ceiling, and a reasoning model will burn the lot thinking and
        // return an empty answer.
        // Streaming is not a UX choice here. A buffered response from a slow model
        // sits silent long enough for the gateway edge to time out mid-generation
        // while the origin is still working — that is what the 524s were, and the
        // gateway will not retry them because the generation is billed either way.
        // Streamed bytes keep the connection alive, so a slow model can finish.
        let mut body = serde_json::json!({
            "model": model_id, "messages": messages, "max_tokens": max_tokens,
            "stream": true, "stream_options": {"include_usage": true}
        });
        // A message with neither content nor tool_calls (a model that returned
        // nothing) serializes without a content field, and strict providers
        // reject the whole request: "message.content must be a string (null is
        // only valid with tool_calls)". Histories already hold such messages,
        // so patch at request time rather than at message creation.
        // Only the newest picture rides the request. Every earlier one was
        // already seen on the turn it arrived, and a history that keeps them
        // all grows past the provider's body limit within a few screenshots:
        // kit-web's second render on 2026-09-02 turned every later call on
        // that run into a 413 and a fall-through to the text-only rung.
        let last_with_images = messages.iter().rposition(|m| m.images.as_ref().is_some_and(|v| !v.is_empty()));
        if let Some(msgs) = body["messages"].as_array_mut() {
            for (i, m) in msgs.iter_mut().enumerate() {
                if m.get("content").is_none() && m.get("tool_calls").is_none() {
                    m["content"] = Value::String(String::new());
                }
                // Pictures become content parts here and nowhere else. Text-only
                // seats get the text alone rather than a 400 — the deliberate
                // ceiling is a substring list, not a capability probe; widen it
                // when a new text-only model joins the ladder.
                if let Some(imgs) = messages.get(i).and_then(|src| src.images.as_ref()) {
                    if !Self::text_only_model(model_id) && !imgs.is_empty() && last_with_images == Some(i) {
                        let text = m["content"].as_str().unwrap_or("").to_string();
                        let mut parts = vec![serde_json::json!({"type": "text", "text": text})];
                        for u in imgs {
                            parts.push(serde_json::json!({"type": "image_url", "image_url": {"url": u}}));
                        }
                        m["content"] = Value::Array(parts);
                    }
                }
            }
        }
        if !tools.is_empty() {
            body["tools"] = Value::Array(tools.to_vec());
        }
        body
    }

    /// Accumulate one streamed completion into the same shape a buffered reply has.
    ///
    /// Returns the message, the usage the final chunk carried, the finish_reason,
    /// and the reasoning-token count — the caller needs the last two together to
    /// tell a real answer from a model that spent its whole budget thinking.
    /// Reads an SSE fill. The last element is why the stream ended early, when
    /// it did: a transport fault, or the gateway's own closing error frame.
    /// A break is no longer a clean failure — the frame carries what the
    /// delivered tokens were billed, so output that reached us is paid for and
    /// the caller must keep it rather than buy the answer twice.
    async fn read_stream(resp: reqwest::Response) -> Result<(Message, Usage, String, u64, Option<String>)> {
        use futures::StreamExt;
        let mut stream = resp.bytes_stream();
        let (mut buf, mut content, mut reasoning, mut finish) =
            (String::new(), String::new(), String::new(), String::new());
        let mut usage = Usage::default();
        let mut reasoning_tokens = 0u64;
        // Tool calls arrive as deltas keyed by index: the id and name land once,
        // then the arguments come a fragment at a time and must be concatenated in
        // arrival order. Anything else produces truncated JSON arguments.
        let mut calls: Vec<(String, String, String)> = Vec::new();
        let mut broke: Option<String> = None;
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(c) => c,
                Err(e) => {
                    broke = Some(format!("stream broke mid-response: {e}"));
                    break;
                }
            };
            buf.push_str(&String::from_utf8_lossy(&chunk));
            // Only whole lines are safe to parse; a chunk can split one in half.
            while let Some(pos) = buf.find('\n') {
                let line: String = buf.drain(..=pos).collect();
                let Some(data) = line.trim().strip_prefix("data:") else { continue };
                let data = data.trim();
                if data.is_empty() || data == "[DONE]" {
                    continue;
                }
                let Ok(v) = serde_json::from_str::<Value>(data) else { continue };
                // The provider stopping mid-answer arrives in-band as an error
                // frame, and its bu0y block says what the delivered tokens cost.
                if let Some(err) = v["error"]["message"].as_str() {
                    broke = Some(format!(
                        "{err} (billed {} micro$ for {} completion tokens)",
                        v["bu0y"]["billed_micros"].as_u64().unwrap_or(0),
                        v["bu0y"]["completion_tokens"].as_u64().unwrap_or(0)
                    ));
                    if let Some(n) = v["bu0y"]["completion_tokens"].as_u64() {
                        usage.completion_tokens = usage.completion_tokens.max(n);
                    }
                    continue;
                }
                // The gateway says whether the route that filled was one it
                // knew met our speed floor. `unverified` means it fell back to
                // an unmeasured route; more than the odd one is worth reporting.
                if let Some(sf) = v["bu0y"]["speed_floor"].as_str() {
                    if sf != "verified" {
                        eprintln!("[llm] speed_floor={sf}");
                    }
                }
                if !v["usage"].is_null() {
                    usage.prompt_tokens = v["usage"]["prompt_tokens"].as_u64().unwrap_or(usage.prompt_tokens);
                    usage.completion_tokens =
                        v["usage"]["completion_tokens"].as_u64().unwrap_or(usage.completion_tokens);
                    reasoning_tokens = v["usage"]["completion_tokens_details"]["reasoning_tokens"]
                        .as_u64()
                        .unwrap_or(reasoning_tokens);
                }
                let ch = &v["choices"][0];
                if let Some(f) = ch["finish_reason"].as_str().filter(|f| !f.is_empty()) {
                    finish = f.to_string();
                }
                let d = &ch["delta"];
                if let Some(c) = d["content"].as_str() {
                    content.push_str(c);
                }
                for key in ["reasoning", "reasoning_content"] {
                    if let Some(r) = d[key].as_str() {
                        reasoning.push_str(r);
                    }
                }
                for tc in d["tool_calls"].as_array().unwrap_or(&Vec::new()) {
                    let idx = tc["index"].as_u64().unwrap_or(0) as usize;
                    while calls.len() <= idx {
                        calls.push((String::new(), String::new(), String::new()));
                    }
                    let slot = &mut calls[idx];
                    if let Some(id) = tc["id"].as_str().filter(|s| !s.is_empty()) {
                        slot.0 = id.to_string();
                    }
                    if let Some(n) = tc["function"]["name"].as_str().filter(|s| !s.is_empty()) {
                        slot.1 = n.to_string();
                    }
                    if let Some(a) = tc["function"]["arguments"].as_str() {
                        slot.2.push_str(a);
                    }
                }
            }
        }
        let tool_calls: Vec<ToolCall> = calls
            .into_iter()
            .filter(|(_, name, _)| !name.is_empty())
            // A call whose argument fragments stopped arriving is truncated JSON.
            // Keeping the partial ANSWER is right — it was paid for; running a
            // half-written tool call is not.
            .filter(|(_, _, args)| {
                broke.is_none() || args.trim().is_empty() || serde_json::from_str::<Value>(args).is_ok()
            })
            .enumerate()
            .map(|(i, (id, name, arguments))| ToolCall {
                // Some gateways omit the id on the delta; the loop below needs a
                // stable one to match the tool result back to its call.
                id: if id.is_empty() { format!("call_{i}") } else { id },
                kind: func_type(),
                function: FunctionCall { name, arguments },
            })
            .collect();
        let msg = Message {
            role: "assistant".into(),
            content: (!content.is_empty()).then_some(content),
            tool_calls: (!tool_calls.is_empty()).then_some(tool_calls),
            tool_call_id: None,
            reasoning: (!reasoning.is_empty()).then_some(reasoning),
            images: None,
        };
        Ok((msg, usage, finish, reasoning_tokens, broke))
    }

    /// Chat completion against any OpenAI-compatible endpoint, with an output
    /// ceiling for answers whose size is known (None = the model's own limit).
    ///
    /// The gateway reserves against the ceiling and refuses when it is larger
    /// than the model can produce at its recent speed, so asking 65,536 tokens
    /// for a summary is not free caution — it is what got two compaction runs
    /// refused, retried into a shrunken budget, and spent entirely on
    /// reasoning on 2026-09-02.
    pub async fn chat_capped(
        &self,
        cfg: &Config,
        model: &str,
        messages: &[Message],
        tools: &[Value],
        cap: Option<u32>,
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
        let max_out = cap.unwrap_or(u32::MAX).min(self.output_limit(model, cfg));
        // The speed floor rides every request to a provider that has one — the
        // shrunken retries below included, which is why it lives in the builder.
        // It is the answer to 2026-09-02: a route decoding at 4.5 tokens/s was
        // the cheapest and so was picked every time, cutting fills at ~128s all day.
        let build = |cap: u32| {
            let mut b = Self::build_request(&model_id, messages, tools, cap);
            if let Some(tps) = prov.min_tokens_per_sec {
                b["min_tokens_per_sec"] = Value::from(tps);
            }
            b
        };
        let mut body = build(max_out);
        // The gateway names a ceiling that fits; take it once and never climb back.
        let mut shrunk = false;
        let mut sent_max = max_out;

        let mut last_err = String::new();
        // A 503 during a bad window means "retry shortly", and the gateway names how
        // long. Honouring that beats guessing at an exponential curve.
        let mut backoff: Option<Duration> = None;
        for attempt in 0..4u32 {
            if attempt > 0 {
                // Announce every retry. Each attempt can block for the full 540s
                // timeout, so without this the loop sits silent for up to ~20
                // minutes and a slow provider is indistinguishable from a hang.
                eprintln!("llm: {model} attempt {}/4 failed, retrying — {last_err}", attempt);
                let wait = backoff
                    .take()
                    .unwrap_or_else(|| Duration::from_secs(2u64.pow(attempt)))
                    .min(Duration::from_secs(30));
                tokio::time::sleep(wait).await;
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
            // A failure is drained as text; a success stays a stream. Buffering the
            // success here would reintroduce the very edge timeout streaming exists
            // to avoid, so every error path is handled before the body is touched.
            if !status.is_success() {
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
                // Carried by a 400 (ask too big to produce inside the fill
                // ceiling at this model's recent speed) and by a 503 below the
                // margin floor. Either way the gateway has done the arithmetic
                // and named the ceiling that fills — re-send at exactly it. The
                // number is recomputed from the speed samples arriving between
                // calls, so a re-send can be refused again with a smaller one
                // (6400 → 5120 → 4608 within 11s on 2026-09-02); follow it down
                // while it keeps shrinking, and let the attempt budget bound it.
                if let Some(fit) = retry_max_tokens(&text).filter(|f| *f < sent_max) {
                    shrunk = true;
                    body = build(fit);
                    sent_max = fit;
                    last_err = format!("{status} asked for a smaller ceiling; retrying at max_tokens {fit}");
                    continue;
                }
                if status.is_server_error() {
                    // An upstream 504/524 means the origin was still generating when
                    // the edge gave up. The request was accepted and may be billed, so
                    // a retry buys a second generation of an answer nobody will read —
                    // and the gateway will not retry it either. Fail out so the caller
                    // can try a DIFFERENT model, which at least produces one usable
                    // answer for the money.
                    if upstream_timeout(&text) {
                        bail!(
                            "{model} upstream timed out mid-generation — not retried, the request may already be billed: {}",
                            text.chars().take(200).collect::<String>()
                        );
                    }
                    // No route clears the speed floor. Retrying the same request
                    // asks the same question; the message names the fastest
                    // speed seen, and the fallback ladder is the relaxation —
                    // another model has its own routes.
                    if unmet_speed(&text) {
                        bail!(
                            "{model}: no route meets the speed floor right now — {}",
                            text.chars().take(240).collect::<String>()
                        );
                    }
                    // 503 is the gateway saying the market is thin right now and to
                    // come back in a moment — a wait, not a fault to diagnose.
                    if status.as_u16() == 503 {
                        backoff = reset;
                    }
                    last_err = format!("{status}: {}", text.chars().take(300).collect::<String>());
                    continue;
                }
                bail!("{model} returned {status}: {}", text.chars().take(500).collect::<String>());
            }
            let (msg, usage, finish, reasoning_tokens, broke) = match Self::read_stream(resp).await {
                Ok(r) => r,
                Err(e) => {
                    last_err = format!("stream: {e:#}");
                    continue;
                }
            };
            if let Some(why) = broke {
                // A stream that broke before delivering anything costs nothing,
                // so retrying it is free. One that delivered output was billed
                // for it: retrying buys the same answer twice and throws the
                // first away. Keep what arrived.
                let delivered = msg.content.is_some() || msg.tool_calls.is_some();
                if !delivered {
                    last_err = format!("stream: {why}");
                    continue;
                }
                eprintln!("llm: {model} stream ended early, keeping the billed partial answer — {why}");
            }
            // A reasoning model can spend its whole output budget thinking and stop
            // before it answers: HTTP 200, empty content, finish_reason "length".
            // Providers say so plainly; not reading it is what made these look like
            // silent empty replies.
            let nothing = msg.content.as_deref().is_none_or(|c| c.trim().is_empty())
                && msg.tool_calls.as_ref().is_none_or(|t| t.is_empty());
            if nothing && finish == "length" {
                return Err(anyhow::Error::new(Truncated { max_tokens: sent_max, reasoning_tokens, gateway_capped: shrunk })
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
