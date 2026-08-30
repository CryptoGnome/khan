use super::ToolCtx;
use anyhow::{bail, Context, Result};
use base64::Engine;
use serde_json::json;

/// The image seat when khan.toml names none: meta/muse-image, $0.01/image flat
/// on OpenRouter's images endpoint. Kept off the bu0y chat pipeline on purpose
/// -- bu0y bills image-model tokens but strips the payload (verified
/// 2026-08-30), so image work routes to OpenRouter alone.
const DEFAULT_MODEL: &str = "openrouter/meta/muse-image";

/// Generate an image via OpenRouter's /images/generations endpoint and save it
/// in the workspace. The provider key never touches an agent's shell
/// environment: the call happens here, in the binary, with the in-memory key.
pub async fn generate(ctx: &ToolCtx, prompt: &str, path: &str, model: &str) -> Result<String> {
    if prompt.trim().is_empty() {
        bail!("prompt is empty");
    }
    if !path.to_ascii_lowercase().ends_with(".png") {
        bail!("path must end in .png (got '{path}')");
    }
    let slug = if model.trim().is_empty() {
        ctx.cfg.image_model.clone().unwrap_or_else(|| DEFAULT_MODEL.into())
    } else if model.contains('/') && !model.starts_with("openrouter/") {
        // Bare OpenRouter ids like "x-ai/grok-imagine-image-2.0" are what the
        // catalog shows; accept them rather than bouncing on a missing prefix.
        format!("openrouter/{model}")
    } else {
        model.to_string()
    };
    let (provider, model_id, key) = ctx.cfg.resolve(&slug)?;
    if provider.name != "openrouter" {
        bail!("image generation runs on the openrouter provider only (got '{}')", provider.name);
    }
    let url = format!("{}/images/generations", provider.base_url.trim_end_matches('/'));
    let body = json!({ "model": model_id, "prompt": prompt, "n": 1 });
    let resp = ctx
        .http
        .post(&url)
        .bearer_auth(&key)
        .json(&body)
        .timeout(std::time::Duration::from_secs(180))
        .send()
        .await
        .context("image request failed")?;
    let status = resp.status();
    let v: serde_json::Value = resp.json().await.context("image response was not JSON")?;
    if !status.is_success() {
        bail!("image request returned {status}: {}", truncate_err(&v));
    }
    let img = &v["data"][0];
    let bytes = if let Some(b64) = img["b64_json"].as_str() {
        base64::engine::general_purpose::STANDARD
            .decode(b64.trim())
            .context("image payload was not valid base64")?
    } else if let Some(u) = img["url"].as_str() {
        // Some models hand back a hosted URL instead of inline base64.
        let b64 = u.split_once("base64,").map(|(_, b)| b.to_string());
        match b64 {
            Some(b) => base64::engine::general_purpose::STANDARD
                .decode(b.trim())
                .context("image payload was not valid base64")?,
            None => ctx
                .http
                .get(u)
                .timeout(std::time::Duration::from_secs(60))
                .send()
                .await
                .context("image URL fetch failed")?
                .bytes()
                .await
                .context("image URL body unreadable")?
                .to_vec(),
        }
    } else {
        bail!("no image in response: {}", truncate_err(&v));
    };
    let written = super::fs::write_binary(ctx, path, &bytes)?;
    let cost = v["usage"]["cost"].as_f64().map(|c| format!(", ${c:.3}")).unwrap_or_default();
    Ok(format!("{written} ({model_id}{cost})"))
}

fn truncate_err(v: &serde_json::Value) -> String {
    v.to_string().chars().take(300).collect()
}
