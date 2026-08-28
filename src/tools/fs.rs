use super::ToolCtx;
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

/// Resolve a workspace-relative path and refuse anything that escapes the workspace.
fn resolve(ctx: &ToolCtx, rel: &str) -> Result<PathBuf> {
    let rel = rel.trim_start_matches(['/', '\\']);
    if Path::new(rel).is_absolute() {
        bail!("absolute paths are not allowed; use paths relative to the workspace");
    }
    let ws = ctx.workspace.canonicalize().context("workspace missing")?;
    let joined = ws.join(rel);
    // Canonicalize the deepest existing ancestor so `..` can't escape.
    let mut probe = joined.clone();
    while !probe.exists() {
        probe = match probe.parent() {
            Some(p) => p.to_path_buf(),
            None => bail!("invalid path"),
        };
    }
    let canon = probe.canonicalize()?;
    if !canon.starts_with(&ws) {
        bail!("path escapes the workspace");
    }
    Ok(joined)
}

pub fn read_file(ctx: &ToolCtx, path: &str) -> Result<String> {
    let p = resolve(ctx, path)?;
    std::fs::read_to_string(&p).with_context(|| format!("cannot read {path}"))
}

pub fn write_file(ctx: &ToolCtx, path: &str, content: &str) -> Result<String> {
    let p = resolve(ctx, path)?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&p, content).with_context(|| format!("cannot write {path}"))?;
    Ok(format!("wrote {} bytes to {path}", content.len()))
}

pub fn list_files(ctx: &ToolCtx, path: &str) -> Result<String> {
    let root = resolve(ctx, path)?;
    let mut out = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for e in entries.flatten() {
            let p = e.path();
            let rel = p.strip_prefix(&root).unwrap_or(&p).display().to_string();
            if p.is_dir() {
                if !rel.contains(".git") && !rel.contains("target") && !rel.contains("node_modules") {
                    stack.push(p);
                }
            } else {
                out.push(rel);
            }
            if out.len() >= 500 {
                out.push("...[more files omitted]".into());
                return Ok(out.join("\n"));
            }
        }
    }
    Ok(if out.is_empty() { "(empty)".into() } else { out.join("\n") })
}
