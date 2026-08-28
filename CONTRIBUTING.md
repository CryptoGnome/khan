# Contributing to khan

Thanks for your interest — contributions are welcome.

## Ground rules

- **Keep it lean.** khan is deliberately small: one binary, few dependencies,
  no frameworks. PRs that add a dependency need a good reason why the stdlib
  or an existing dependency can't do it.
- **Match the existing style.** Plain Rust, minimal abstractions, comments
  only where the code can't explain itself.
- **Don't weaken the security layers.** The immutable prompt preamble, env
  scrubbing, `gh` blocking, and the read-only log viewer are load-bearing.
  Changes touching them get extra scrutiny (see [SECURITY.md](SECURITY.md)).

## Getting started

```bash
git clone https://github.com/CryptoGnome/khan.git
cd khan
cp khan.toml.example khan.toml   # add your models / providers
cargo build
cargo test
```

You'll need at least one API key exported (`OPENROUTER_API_KEY` or
`BU0Y_API_KEY`) to actually run it; `cargo test` works without any.

## Submitting changes

1. Fork, branch from `main`.
2. Make the change; add or update tests where behavior changed
   (`cargo test` must pass).
3. Keep commits focused — one logical change per PR.
4. Open a PR describing what changed and why.

For larger features, open an issue first to discuss the direction — it saves
everyone time.

## Reporting bugs

Open an issue with: what you ran (redact your directive if private), what you
expected, what happened, and relevant log lines from the terminal or the web
log viewer. For security issues, use
[private reporting](https://github.com/CryptoGnome/khan/security/advisories/new)
instead — see [SECURITY.md](SECURITY.md).
