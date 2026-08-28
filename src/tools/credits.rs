use super::ToolCtx;
use anyhow::{Context, Result};
use serde_json::{json, Value};

/// Provider whose prepaid balance agents can inspect. Agents never see the API
/// key — it is read here, used for one request, and only the response is returned.
const PROVIDER: &str = "bu0y";

pub fn schemas(ctx: &ToolCtx) -> Vec<Value> {
    if !ctx.cfg.providers.iter().any(|p| p.name == PROVIDER) {
        return vec![];
    }
    vec![json!({"type": "function", "function": {
        "name": "credits",
        "description": "Check the company's prepaid AI credit balance and recent usage, and get the USDC deposit address for topping it up. Use this before planning expensive model work, and to decide when the treasury needs to buy more credits.",
        "parameters": {"type": "object", "properties": {}, "required": []}}})]
}

pub async fn run(ctx: &ToolCtx) -> Result<String> {
    let prov = ctx
        .cfg
        .providers
        .iter()
        .find(|p| p.name == PROVIDER)
        .with_context(|| format!("no '{PROVIDER}' provider configured"))?;
    let key = ctx
        .cfg
        .key_for(PROVIDER)
        .with_context(|| format!("env var {} not set", prov.api_key_env))?
        .to_string();
    let base = prov.base_url.trim_end_matches('/');

    let get = |path: String| {
        let req = ctx.http.get(path).bearer_auth(&key);
        async move {
            match req.send().await {
                Ok(r) => {
                    let status = r.status();
                    let body = r.text().await.unwrap_or_default();
                    format!("[{status}] {}", body.chars().take(2000).collect::<String>())
                }
                Err(e) => format!("request failed: {e}"),
            }
        }
    };

    let usage = get(format!("{base}/account/usage")).await;
    let deposit = get(format!("{base}/deposits/solana")).await;
    Ok(format!(
        "--- credit balance & usage ---\n{usage}\n\n--- Solana USDC deposit address (send USDC here to top up; \
min $1; credits are prepaid usage rights, non-refundable and non-withdrawable) ---\n{deposit}"
    ))
}
