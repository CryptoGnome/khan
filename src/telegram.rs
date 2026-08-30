use crate::state::Store;
use std::sync::Arc;

/// Direct line between founder and CEO. Inbound: long-poll the Telegram Bot
/// API and queue messages from the founder's chat into the same queue as
/// `khan tell`, so the CEO wakes on them as founder input. Anything from any
/// other chat is dropped and logged — this channel carries founder authority,
/// so it is bound to one allowlisted chat id, never first-come-first-bound.
/// Outbound lives in agent.rs as the CEO's message_founder tool.
pub async fn serve(store: Arc<Store>, http: reqwest::Client, token: String, chat_id: i64) {
    let base = format!("https://api.telegram.org/bot{token}");
    // The update offset survives restarts in kv: a deploy mid-conversation
    // must not replay (or drop) founder messages.
    let mut offset: i64 = store.kv_get("telegram_offset").and_then(|v| v.parse().ok()).unwrap_or(0);
    store.log("core", "telegram", "founder line up (long-polling)");
    loop {
        let url = format!("{base}/getUpdates?timeout=50&offset={offset}");
        let resp = http
            .get(&url)
            .timeout(std::time::Duration::from_secs(70))
            .send()
            .await;
        let v: serde_json::Value = match resp {
            Ok(r) => match r.json().await {
                Ok(v) => v,
                Err(_) => {
                    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                    continue;
                }
            },
            Err(_) => {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                continue;
            }
        };
        for upd in v["result"].as_array().into_iter().flatten() {
            if let Some(id) = upd["update_id"].as_i64() {
                offset = offset.max(id + 1);
            }
            let msg = &upd["message"];
            let (Some(from), Some(text)) = (msg["chat"]["id"].as_i64(), msg["text"].as_str())
            else {
                continue;
            };
            if from != chat_id {
                store.log("core", "telegram", &format!("dropped message from unknown chat {from}"));
                continue;
            }
            store.log("core", "telegram", "founder message received");
            store.queue_message(&format!(
                "[via Telegram] {text}\n(The founder sent this from their phone — answer with the \
                 message_founder tool so the reply reaches them there.)"
            ));
        }
        store.kv_set("telegram_offset", &offset.to_string());
    }
}

/// Send one message to the founder's Telegram chat. Telegram caps messages at
/// 4096 chars; longer text is truncated rather than bounced.
pub async fn send(http: &reqwest::Client, token: &str, chat_id: i64, text: &str) -> Result<(), String> {
    let text: String = text.chars().take(4000).collect();
    let url = format!("https://api.telegram.org/bot{token}/sendMessage");
    let resp = http
        .post(&url)
        .json(&serde_json::json!({"chat_id": chat_id, "text": text}))
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| format!("telegram send failed: {e}"))?;
    if resp.status().is_success() {
        Ok(())
    } else {
        let s = resp.status();
        let body = resp.text().await.unwrap_or_default();
        Err(format!("telegram returned {s}: {}", body.chars().take(200).collect::<String>()))
    }
}
