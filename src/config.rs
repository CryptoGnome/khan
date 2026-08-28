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
    /// Model used for cheap internal work (history summarization). Defaults to ceo_model.
    pub utility_model: Option<String>,
    #[serde(default = "default_workspace")]
    pub workspace: String,
    pub providers: Vec<Provider>,
    /// Reflection/evolution runs every this many CEO iterations.
    #[serde(default = "default_reflect_every")]
    pub reflect_every: u64,
    /// Max agent-loop iterations for a delegated employee task.
    #[serde(default = "default_employee_max_iters")]
    pub employee_max_iters: u64,
    /// Provider API keys, read from the environment once at load and held only
    /// here. Never deserialized from the config file — keys never live on disk.
    #[serde(skip)]
    keys: std::collections::HashMap<String, String>,
}

fn default_workspace() -> String {
    "workspace".into()
}
fn default_reflect_every() -> u64 {
    10
}
fn default_employee_max_iters() -> u64 {
    30
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
