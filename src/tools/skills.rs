use super::ToolCtx;
use anyhow::{bail, Result};
use serde_json::{json, Value};

/// Schemas for the skill-library tools (available to every agent).
pub fn schemas() -> Vec<Value> {
    vec![
        json!({"type": "function", "function": {"name": "create_skill",
            "description": "Create a NEW skill, or improve an existing one (same name = new version, old versions kept). A skill is a reusable how-to document (markdown) for a procedure the company does often — steps, gotchas, checklists, examples. Every agent sees the skill index and loads a skill with use_skill when relevant.",
            "parameters": {"type": "object", "properties": {
                "name": {"type": "string", "description": "snake_case skill name"},
                "description": {"type": "string", "description": "One line: when an agent should load this skill"},
                "content": {"type": "string", "description": "The full skill: instructions, steps, gotchas, examples (markdown)"},
                "reason": {"type": "string", "description": "Why this skill / this improvement"}},
                "required": ["name", "description", "content", "reason"]}}}),
        json!({"type": "function", "function": {"name": "use_skill",
            "description": "Load a skill's full instructions into context. Do this before starting work the skill index says it covers.",
            "parameters": {"type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"]}}}),
        json!({"type": "function", "function": {"name": "rollback_skill",
            "description": "Revert a skill to its previous version (use if an improvement made it worse).",
            "parameters": {"type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"]}}}),
    ]
}

/// Seed curated skills from the repo's skills/ directory (baked into the
/// image). Each file is <name>.md: first line the one-line description, the
/// rest the content. A changed file ships as a new version only while the
/// skill's latest version is still seed-origin; anything the company has
/// since written itself is never overridden — agents evolve and roll back
/// seeded skills exactly like their own.
pub fn seed(store: &crate::state::Store) {
    // The container runs with WORKDIR /data (the volume) while the image
    // bakes the seeds at /app/skills — the first deploy seeded nothing
    // because only the relative path was tried. Local dev still hits ./skills.
    let dir = ["skills", "/app/skills"]
        .iter()
        .map(std::path::Path::new)
        .find(|p| p.is_dir());
    let Some(dir) = dir else { return };
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let mut lines = text.splitn(2, '\n');
        let (desc, content) = (lines.next().unwrap_or("").trim(), lines.next().unwrap_or("").trim());
        if desc.is_empty() || content.is_empty() {
            continue;
        }
        // An improved seed file ships as a new version — but only while the
        // skill's latest version is still seed-origin. The moment the company
        // writes its own version, the file becomes a dead letter by design.
        if let Some((cur, reason)) = store.skill_latest_meta(name) {
            if !reason.starts_with("seeded") || cur == content {
                continue;
            }
        }
        if store.save_skill(name, desc, content, "seeded from the repo's skills/ directory").is_ok() {
            store.log("core", "skill-seeded", &format!("{name}: {desc}"));
        }
    }
}

pub fn create(ctx: &ToolCtx, args: &Value) -> Result<String> {
    let name = args["name"].as_str().unwrap_or("").trim().to_string();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        bail!("skill name must be non-empty snake_case (ascii letters, digits, underscores)");
    }
    let desc = args["description"].as_str().unwrap_or("").trim();
    let content = args["content"].as_str().unwrap_or("").trim();
    if desc.is_empty() || content.is_empty() {
        bail!("description and content must not be empty");
    }
    let v = ctx.store.save_skill(&name, desc, content, args["reason"].as_str().unwrap_or(""))?;
    Ok(if v == 1 {
        format!("skill '{name}' created — it now appears in every agent's skill index")
    } else {
        format!("skill '{name}' updated to version {v} (rollback_skill reverts)")
    })
}

pub fn load(ctx: &ToolCtx, agent: &str, name: &str) -> String {
    match ctx.store.get_skill(name) {
        Some((desc, content)) => {
            // Loads feed the outcome stats: each is joined to the loader's next
            // rating so reflection judges skills on results, like prompts.
            ctx.store.log_skill_load(agent, name);
            format!("# Skill: {name}\n{desc}\n\n{content}")
        }
        None => format!("no such skill '{name}' — check the skill index"),
    }
}

/// Compact index injected into agent context each turn (None when no skills exist).
pub fn index(ctx: &ToolCtx) -> Option<String> {
    let skills = ctx.store.list_skills();
    if skills.is_empty() {
        return None;
    }
    let list: Vec<String> = skills.iter().map(|(n, d)| format!("- {n}: {d}")).collect();
    Some(format!(
        "[Skill library — load one with use_skill(name) before doing work it covers]\n{}",
        list.join("\n")
    ))
}
