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

pub fn load(ctx: &ToolCtx, name: &str) -> String {
    match ctx.store.get_skill(name) {
        Some((desc, content)) => format!("# Skill: {name}\n{desc}\n\n{content}"),
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
