use crate::config::Config;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Message {
    pub fn text(role: &str, content: impl Into<String>) -> Message {
        Message { role: role.into(), content: Some(content.into()), tool_calls: None, tool_call_id: None }
    }
    pub fn tool_result(id: &str, content: impl Into<String>) -> Message {
        Message { role: "tool".into(), content: Some(content.into()), tool_calls: None, tool_call_id: Some(id.into()) }
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

pub struct Client {
    http: reqwest::Client,
}

impl Client {
    pub fn new() -> Client {
        Client {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(300))
                .build()
                .expect("http client"),
        }
    }

    pub fn build_request(model_id: &str, messages: &[Message], tools: &[Value]) -> Value {
        let mut body = serde_json::json!({ "model": model_id, "messages": messages });
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
        let url = format!("{}/chat/completions", prov.base_url.trim_end_matches('/'));
        let body = Self::build_request(&model_id, messages, tools);

        let mut last_err = String::new();
        for attempt in 0..4u32 {
            if attempt > 0 {
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
            let text = resp.text().await.unwrap_or_default();
            if status.as_u16() == 429 || status.is_server_error() {
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
            return Ok((msg, usage));
        }
        bail!("{model} failed after retries: {last_err}")
    }
}
