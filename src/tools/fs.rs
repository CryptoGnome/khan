use super::ToolCtx;
use anyhow::{bail, Context, Result};
use std::path::{Component, Path, PathBuf};

/// Resolve a workspace-relative path and refuse anything that escapes the workspace.
fn resolve(ctx: &ToolCtx, rel: &str) -> Result<PathBuf> {
    let rel = rel.trim_start_matches(['/', '\\']);
    if Path::new(rel).is_absolute() {
        bail!("absolute paths are not allowed; use paths relative to the workspace");
    }
    // Reject `..` outright. The ancestor probe below cannot be trusted to catch it:
    // on Linux, `exists()` fails on any path with a missing component, so a path like
    // `new/../../khan.db` rewinds the probe all the way back to the workspace, passes
    // the containment check, and then write_file's create_dir_all materialises `new`
    // and the write lands outside. (Windows normalises `..` lexically and blocks it,
    // so this only ever bit the Linux deployment.)
    if Path::new(rel).components().any(|c| matches!(c, Component::ParentDir)) {
        bail!("`..` is not allowed in workspace paths");
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

/// Write a workspace file, overwriting it or appending to the end.
///
/// Appending exists because a single model response has a hard output ceiling: a
/// large file cannot arrive as one tool argument, so it has to be built across
/// several turns. Without this an agent's only route was a shell heredoc.
pub fn write_file(ctx: &ToolCtx, path: &str, content: &str, append: bool) -> Result<String> {
    let p = resolve(ctx, path)?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if !append {
        std::fs::write(&p, content).with_context(|| format!("cannot write {path}"))?;
        return Ok(format!("wrote {} bytes to {path}", content.len()));
    }
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&p)
        .with_context(|| format!("cannot open {path} to append"))?;
    f.write_all(content.as_bytes()).with_context(|| format!("cannot append to {path}"))?;
    // Report the running total: the agent is building towards a size it has in
    // mind and otherwise cannot tell how far along it is.
    let total = std::fs::metadata(&p).map(|m| m.len()).unwrap_or_default();
    Ok(format!("appended {} bytes to {path} ({total} bytes total)", content.len()))
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
