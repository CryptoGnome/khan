use super::ToolCtx;
use anyhow::{bail, Context, Result};
use base64::Engine;
use serde_json::{json, Value};

/// X (Twitter) posting via the founder's developer-portal app, OAuth 2.0
/// user-context (the recommended path; 1.0a is legacy). The confidential
/// client (X_CLIENT_ID / X_CLIENT_SECRET) rides cfg.keys; the rotating
/// refresh token is seeded once from X_REFRESH_TOKEN and thereafter lives in
/// kv, because X invalidates each refresh token when it is used — losing the
/// rotated value means a new browser authorization, so it is persisted before
/// the access token is used for anything.
const KV_REFRESH: &str = "x_refresh_token";

pub fn schemas(ctx: &ToolCtx) -> Vec<Value> {
    // The tool only exists when the founder has configured the client and a
    // refresh token exists (env seed or a previously rotated one in kv) —
    // structural enforcement: no credentials, no X surface to reason about.
    let configured = ctx.cfg.secret("X_CLIENT_ID").is_some()
        && ctx.cfg.secret("X_CLIENT_SECRET").is_some()
        && (ctx.cfg.secret("X_REFRESH_TOKEN").is_some() || ctx.store.kv_get(KV_REFRESH).is_some());
    if !configured {
        return vec![];
    }
    vec![json!({"type": "function", "function": {
        "name": "x_post",
        "description": "Post to the company's X (Twitter) account via the official API. Load the farcaster_voice_policy skill FIRST — the same rules govern X: post only on real events, a few a day max, no shilling, no return promises. Optionally reply to a tweet by id.",
        "parameters": {"type": "object", "properties": {
            "text": {"type": "string", "description": "The post text (280 chars max)"},
            "reply_to": {"type": "string", "description": "Optional tweet id to reply to"}},
            "required": ["text"]}}})]
}

/// Exchange the current refresh token for an access token, persisting the
/// rotated refresh token BEFORE returning — the old one is dead the moment
/// the exchange succeeds.
async fn access_token(ctx: &ToolCtx) -> Result<String> {
    let id = ctx.cfg.secret("X_CLIENT_ID").context("X_CLIENT_ID not configured")?;
    let secret = ctx.cfg.secret("X_CLIENT_SECRET").context("X_CLIENT_SECRET not configured")?;
    // kv (a rotated token) beats the env seed: the seed is only valid until
    // its first use.
    let refresh = ctx
        .store
        .kv_get(KV_REFRESH)
        .or_else(|| ctx.cfg.secret("X_REFRESH_TOKEN").map(String::from))
        .context("no X refresh token available")?;
    let basic = base64::engine::general_purpose::STANDARD.encode(format!("{id}:{secret}"));
    let resp = ctx
        .http
        .post("https://api.x.com/2/oauth2/token")
        .header("Authorization", format!("Basic {basic}"))
        .form(&[("grant_type", "refresh_token"), ("refresh_token", refresh.as_str())])
        .send()
        .await
        .context("token refresh request failed")?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!(
            "x token refresh returned {status}: {} — if this says invalid_request/invalid_grant the \
refresh token chain is broken and the founder must re-authorize the app (do NOT retry in a loop)",
            body.chars().take(400).collect::<String>()
        );
    }
    let v: Value = serde_json::from_str(&body).context("token response not json")?;
    let access = v["access_token"].as_str().context("no access_token in response")?.to_string();
    if let Some(new_refresh) = v["refresh_token"].as_str() {
        ctx.store.kv_set(KV_REFRESH, new_refresh);
    }
    Ok(access)
}

pub async fn post(ctx: &ToolCtx, text: &str, reply_to: &str) -> Result<String> {
    let text = text.trim();
    if text.is_empty() {
        bail!("post text must not be empty");
    }
    if text.chars().count() > 280 {
        bail!("post is {} chars — X caps at 280; cut it down", text.chars().count());
    }
    let token = access_token(ctx).await?;
    let mut body = json!({"text": text});
    if !reply_to.trim().is_empty() {
        body["reply"] = json!({"in_reply_to_tweet_id": reply_to.trim()});
    }
    let resp = ctx
        .http
        .post("https://api.x.com/2/tweets")
        .bearer_auth(&token)
        .json(&body)
        .send()
        .await
        .context("x api request failed")?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        // The body names the reason (duplicate, permissions, rate limit) — hand
        // it to the agent verbatim; a blind retry is how duplicate-post
        // incidents happen (the Farcaster lesson applies here too).
        bail!(
            "x api returned {status}: {} — do NOT blind-retry; verify before any resend",
            body.chars().take(600).collect::<String>()
        );
    }
    let v: Value = serde_json::from_str(&body).unwrap_or_default();
    let id = v["data"]["id"].as_str().unwrap_or("?");
    Ok(format!("posted — tweet id {id}. Verify it renders on the account page before casting follow-ups."))
}
