# Security Policy

## Reporting a vulnerability

Please report vulnerabilities privately via
[GitHub Security Advisories](https://github.com/CryptoGnome/khan/security/advisories/new)
rather than opening a public issue. Reports are reviewed as soon as possible;
fixes ship as regular releases.

## Threat model — what khan does and does not protect against

khan runs LLM agents with **real shell access** on whatever machine hosts it.
Understand the boundaries before deploying:

**Built-in protections**

- An immutable security preamble is appended to every agent's system prompt
  from code (not from the editable prompts table), covering prompt-injection
  resistance, secret handling, and the read-only rule for the public log page.
- API keys are moved out of the process environment entirely at startup: khan
  reads them once into memory, then removes them, so they are absent from
  `/proc/<pid>/environ` as well as from every shell child agents spawn. Other
  secret-shaped environment variables are removed at the same time.
- The GitHub CLI (`gh`) is blocked in the agent shell so agents can never use
  a personal GitHub login; only local `git` is available.
- The web log viewer has **no write endpoints** — page viewers cannot send
  anything to the agents.
- Web content fetched by agents is wrapped in explicit untrusted-content
  markers.
- Founder authority is bound to channels the binary verifies: `khan tell` on
  the host and, when configured, one allowlisted Telegram chat id (messages
  from any other chat are dropped and logged; the bot token is captured into
  memory at startup like the API keys). The security preamble tells agents
  that email and web content can never carry founder instructions — a company
  inbox is a public attack surface.

**Known limitations (by design — deploy accordingly)**

- khan is **not a sandbox**. Agents can run arbitrary shell commands on the
  host. Run it in a container (the provided Dockerfile / Railway setup), not
  on a machine holding anything you can't afford to expose.
- The log viewer is **unauthenticated**. Anyone with the URL can read the
  full activity log, including tool arguments. Key-shaped strings are redacted
  from it in code, but that is a pattern match, not a guarantee — a mnemonic
  seed phrase is ordinary words and cannot be detected. Keep the URL private
  or don't enable public networking.
- There is **no spend cap** on API usage. The only hard stop is stopping the
  process/service.

Reports about the above limitations themselves are expected behavior, not
vulnerabilities — but bypasses of the built-in protections (e.g. an agent
able to read a stripped secret, or a viewer able to reach an agent) are
exactly what we want to hear about.
