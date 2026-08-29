use super::ToolCtx;
use anyhow::{bail, Context, Result};
use std::path::{Component, Path, PathBuf};

/// Resolve a workspace-relative path and refuse anything that escapes the workspace.
fn resolve(ctx: &ToolCtx, rel: &str) -> Result<PathBuf> {
    // Agents habitually pass the file's absolute path. When it points inside the
    // workspace, accept it as the workspace-relative path it means — the old
    // slash-stripping turned "/data/workspace/x" into "<ws>/data/workspace/x",
    // a phantom ENOENT on a file that exists.
    let ws_abs = ctx.workspace.canonicalize().context("workspace missing")?;
    let rel = Path::new(rel)
        .strip_prefix(&ws_abs)
        .ok()
        .and_then(|p| p.to_str())
        .unwrap_or(rel);
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
    let ws = ws_abs;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolCtx;
    use std::sync::Arc;

    fn ctx(ws: &Path) -> ToolCtx {
        ToolCtx {
            cfg: toml::from_str(concat!(
                "ceo_model = \"test/model\"\n",
                "[[providers]]\nname = \"test\"\nbase_url = \"http://localhost\"\napi_key_env = \"TEST_KEY\"\n",
            )).unwrap(),
            store: Arc::new(crate::state::Store::open(":memory:").unwrap()),
            workspace: ws.to_path_buf(),
            http: reqwest::Client::new(),
            http_proxy: None,
        }
    }

    #[test]
    fn absolute_path_inside_workspace_reads_the_file() {
        let dir = std::env::temp_dir().join("khan-fs-test");
        std::fs::create_dir_all(dir.join("vault")).unwrap();
        std::fs::write(dir.join("vault/k.json"), "ok").unwrap();
        let c = ctx(&dir);
        // The shape agents actually send: the file's own absolute path.
        let abs = dir.canonicalize().unwrap().join("vault").join("k.json");
        assert_eq!(read_file(&c, abs.to_str().unwrap()).unwrap(), "ok");
        // Workspace-relative still works.
        assert_eq!(read_file(&c, "vault/k.json").unwrap(), "ok");
        // Absolute paths OUTSIDE the workspace are still refused.
        assert!(read_file(&c, "/etc/hostname").is_err());
    }
}
