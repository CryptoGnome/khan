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
        let cfg: Config = toml::from_str(&text).context("invalid khan.toml")?;
        if cfg.providers.is_empty() {
            bail!("khan.toml must define at least one [[providers]] entry");
        }
        cfg.resolve(&cfg.ceo_model)?; // validate early
        Ok(cfg)
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
        let key = std::env::var(&prov.api_key_env)
            .with_context(|| format!("env var {} not set (needed by provider {})", prov.api_key_env, prov.name))?;
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
