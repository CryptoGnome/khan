use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize, Clone)]
pub struct Provider {
    pub name: String,
    pub base_url: String,
    pub api_key_env: String,
    /// Models offered by this provider, referenced as "<provider>/<model>".
    #[serde(default)]
    pub free_models: Vec<String>,
    #[serde(default)]
    pub paid_models: Vec<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    /// Model the CEO runs on, as "provider/model".
    pub ceo_model: String,
    /// Quality-ordered seat ladder for the CEO. The binary — never the model —
    /// picks the first entry that is not benched by a recent failure and whose
    /// live marketplace price fits the ceilings below. `ceo_model` stays the
    /// always-available floor.
    #[serde(default)]
    pub ceo_models: Vec<String>,
    /// Price ceilings for the seat ladder, in the provider catalog's per-1M
    /// units (the `input.average` / `output.average` fields). A ladder model
    /// currently priced above either ceiling is skipped this cycle — and picked
    /// up again the moment its book improves (opus has traded below grok).
    #[serde(default = "default_ceo_max_input_price")]
    pub ceo_max_input_price: u64,
    #[serde(default = "default_ceo_max_output_price")]
    pub ceo_max_output_price: u64,
    /// Model used for cheap internal work (history summarization). Defaults to ceo_model.
    pub utility_model: Option<String>,
    /// Seat for quiet-board heartbeats: when a heartbeat fires with NOTHING
    /// queued (no reports, no alerts, no founder messages), the episode runs on
    /// this cheaper model instead of the ladder pick. The escalation trigger is
    /// mechanical — a queue check by the binary — so no weak model ever judges
    /// whether a strong one is needed. Unset = every episode uses the ladder.
    pub heartbeat_model: Option<String>,
    /// Model for the generate_image tool, in provider/model form. OpenRouter
    /// only — the OpenRouter key is spent on image generation and nothing else;
    /// chat stays on bu0y. Unset = the tool's built-in cheap default.
    pub image_model: Option<String>,
    /// Founder's standing model policy, injected verbatim into the CEO's
    /// system prompt every episode. Before this it lived only in nudges and
    /// the CEO's playbook — a restart forgot it within the hour (2026-08-30:
    /// fresh boot, new hires drifted straight back to deepseekv4flash).
    pub model_policy: Option<String>,
    /// Low-fuel floor in provider micro-dollars (bu0y GET /account
    /// availableMicros). Below it the binary files an hourly "fuel-low" routine
    /// alert so the CEO tops up before calls start bouncing with 402. 0 disables.
    #[serde(default = "default_fuel_low_micros")]
    pub fuel_low_micros: u64,
    #[serde(default = "default_workspace")]
    pub workspace: String,
    pub providers: Vec<Provider>,
    /// Seconds of event silence before the CEO takes a proactive strategy
    /// (reflection) turn anyway. The event-driven loop otherwise runs only when
    /// something happens; reflection rides on these heartbeat episodes.
    #[serde(default = "default_heartbeat_secs")]
    pub heartbeat_secs: u64,
    /// Turn cap per CEO episode: the backstop when it never calls finish_episode.
    #[serde(default = "default_episode_max_steps")]
    pub episode_max_steps: u64,
    /// Max agent-loop iterations for a delegated employee task.
    #[serde(default = "default_employee_max_iters")]
    pub employee_max_iters: u64,
    /// Output ceiling sent on every request. Must be set explicitly: gateways
    /// impose a small default (bu0y uses 4096) when it is omitted, and a
    /// reasoning model can spend that entire budget thinking and return an empty
    /// answer with finish_reason "length".
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    /// Provider API keys, read from the environment once at load and held only
    /// here. Never deserialized from the config file — keys never live on disk.
    #[serde(skip)]
    keys: std::collections::HashMap<String, String>,
}

fn default_workspace() -> String {
    "workspace".into()
}

fn default_ceo_max_input_price() -> u64 {
    4_000_000
}

fn default_ceo_max_output_price() -> u64 {
    12_000_000
}

fn default_fuel_low_micros() -> u64 {
    10_000_000 // $10 — a few hours of top-seat burn
}

fn default_heartbeat_secs() -> u64 {
    300
}

fn default_episode_max_steps() -> u64 {
    12
}
#[allow(dead_code)]
fn default_reflect_every() -> u64 {
    10
}
fn default_employee_max_iters() -> u64 {
    30
}

fn default_max_tokens() -> u32 {
    16_384
}

impl Config {
    pub fn load(path: &str) -> Result<Config> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read config file {path} (copy khan.toml.example to khan.toml)"))?;
        let mut cfg: Config = toml::from_str(&text).context("invalid khan.toml")?;
        if cfg.providers.is_empty() {
            bail!("khan.toml must define at least one [[providers]] entry");
        }
        cfg.take_keys_from_env();
        cfg.resolve(&cfg.ceo_model)?; // validate early
        for m in &cfg.ceo_models {
            cfg.resolve(m).with_context(|| format!("ceo_models entry '{m}' does not resolve"))?;
        }
        if let Some(m) = &cfg.heartbeat_model {
            cfg.resolve(m).with_context(|| format!("heartbeat_model '{m}' does not resolve"))?;
        }
        if let Some(m) = &cfg.image_model {
            cfg.resolve(m).with_context(|| format!("image_model '{m}' does not resolve"))?;
        }
        Ok(cfg)
    }

    /// Move API keys out of the process environment and into this struct.
    ///
    /// Stripping secrets from agent-spawned children (tools::shell) is not enough
    /// on its own: khan's own environment still held the keys, and on Linux a
    /// child can read its parent's copy straight out of /proc/<pid>/environ,
    /// walking around the whole control. Whether that read is permitted depends
    /// on the host kernel's yama ptrace_scope — which is not ours to rely on — so
    /// take the keys out of the environment entirely and keep them in memory.
    ///
    /// Called once during load, before any agent task exists.
    fn take_keys_from_env(&mut self) {
        for p in &self.providers {
            if let Ok(v) = std::env::var(&p.api_key_env) {
                self.keys.insert(p.api_key_env.clone(), v);
            }
        }
        // The Telegram founder line: the token matches the sensitive pattern and
        // would be scrubbed below, so capture it (and the chat id, for symmetry)
        // into memory first.
        // The founder-provided X (Twitter) OAuth 2.0 credentials ride the same
        // path: captured here, used only inside the x_post tool, never visible
        // to an agent shell (the purchased-account era ended with a
        // founder-owned developer-portal app instead). X_REFRESH_TOKEN is a
        // one-time seed — rotation lives in kv after first use.
        for k in [
            "TELEGRAM_BOT_TOKEN",
            "TELEGRAM_CHAT_ID",
            "GITHUB_TOKEN",
            "X_CLIENT_ID",
            "X_CLIENT_SECRET",
            "X_REFRESH_TOKEN",
            // app-only bearer for the Activity stream (optional; the
            // client_credentials grant is the fallback when unset)
            "X_BEARER_TOKEN",
        ] {
            if let Ok(v) = std::env::var(k) {
                self.keys.insert(k.into(), v);
            }
        }
        // Drop the provider keys plus anything else secret-shaped: khan needs none
        // of them from the environment again, and what isn't there cannot leak.
        let doomed: Vec<String> = std::env::vars()
            .map(|(k, _)| k)
            .filter(|k| crate::tools::shell::is_sensitive_env(k) || self.keys.contains_key(k))
            .collect();
        for k in doomed {
            std::env::remove_var(k);
        }
    }

    /// The founder's Telegram line, when both halves are configured:
    /// (bot token, allowlisted founder chat id).
    pub fn telegram(&self) -> Option<(String, i64)> {
        let token = self.keys.get("TELEGRAM_BOT_TOKEN")?.clone();
        let chat = self.keys.get("TELEGRAM_CHAT_ID")?.parse().ok()?;
        Some((token, chat))
    }

    /// A captured non-provider secret by env-var name (X credentials etc.),
    /// or None if it was never set.
    pub fn secret(&self, name: &str) -> Option<&str> {
        self.keys.get(name).map(String::as_str)
    }

    /// The API key for a provider, or None if it was never set.
    pub fn key_for(&self, provider: &str) -> Option<&str> {
        let p = self.providers.iter().find(|p| p.name == provider)?;
        self.keys.get(&p.api_key_env).map(String::as_str)
    }

    /// Split "provider/model" into (provider, model id, api key).
    pub fn resolve(&self, model: &str) -> Result<(Provider, String, String)> {
        let (pname, mname) = model
            .split_once('/')
            .with_context(|| format!("model '{model}' must be in provider/model form"))?;
        let prov = self
            .providers
            .iter()
            .find(|p| p.name == pname)
            .with_context(|| format!("unknown provider '{pname}' in model '{model}'"))?;
        // Read from the in-memory map, not the environment: take_keys_from_env has
        // already removed these vars from the process so nothing can read them back.
        let key = self
            .keys
            .get(&prov.api_key_env)
            .with_context(|| format!("env var {} not set (needed by provider {})", prov.api_key_env, prov.name))?
            .clone();
        Ok((prov.clone(), mname.to_string(), key))
    }

    pub fn utility_model(&self) -> String {
        self.utility_model.clone().unwrap_or_else(|| self.ceo_model.clone())
    }

    pub fn workspace_dir(&self) -> PathBuf {
        PathBuf::from(&self.workspace)
    }

    /// All free models as "provider/model" ids, used for automatic failover.
    pub fn free_model_ids(&self) -> Vec<String> {
        self.providers
            .iter()
            .flat_map(|p| p.free_models.iter().map(move |m| format!("{}/{}", p.name, m)))
            .collect()
    }

    pub fn paid_model_ids(&self) -> Vec<String> {
        self.providers
            .iter()
            .flat_map(|p| p.paid_models.iter().map(move |m| format!("{}/{}", p.name, m)))
            .collect()
    }

    /// Models to try after `model` fails, preserving the tier it was hired at.
    ///
    /// Putting an agent on a free model is already a judgement that the work is
    /// cheap enough for one, so a free agent keeps the free ladder. Anything paid
    /// falls back only to paid models: the old ladder sent everyone to the free
    /// list on any failure, which silently demoted a job someone had deliberately
    /// chosen an expensive model for, and nobody was told — the CEO just received
    /// a worse report. For the CEO itself it landed at precisely the moments a
    /// decision mattered, since a fallback only happens when the primary failed.
    pub fn fallback_ids_for(&self, model: &str) -> Vec<String> {
        let is_free = self
            .providers
            .iter()
            .any(|p| p.free_models.iter().any(|m| format!("{}/{}", p.name, m) == model));
        if is_free {
            self.free_model_ids()
        } else {
            self.paid_model_ids()
        }
    }

    /// Catalog models with no measured history yet.
    ///
    /// An evidence-driven policy cannot reach these on its own: a model with zero
    /// calls loses every comparison to one with thousands of good ones, however
    /// capable it is, and the gap never closes because nothing generates the first
    /// data point. Naming them is what makes the choice available.
    pub fn untried_models(&self, seen: &[String]) -> Vec<String> {
        self.paid_model_ids()
            .into_iter()
            .chain(self.free_model_ids())
            .filter(|m| !seen.iter().any(|s| s == m))
            .collect()
    }

    /// Human-readable model catalog for prompts.
    pub fn model_catalog(&self) -> String {
        let mut s = String::new();
        for p in &self.providers {
            for m in &p.free_models {
                s.push_str(&format!("- {}/{} (FREE)\n", p.name, m));
            }
            for m in &p.paid_models {
                s.push_str(&format!("- {}/{} (paid)\n", p.name, m));
            }
        }
        s
    }
}
