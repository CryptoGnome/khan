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
  resistance, secret handling, the read-only rule for the public log page,
  and the one-server rule: the company never acquires outside hosting or
  migrates off the server the founder provided.
- API keys are moved out of the process environment entirely at startup: khan
  reads them once into memory, then removes them, so they are absent from
  `/proc/<pid>/environ` as well as from every shell child agents spawn. Other
  secret-shaped environment variables are removed at the same time.
- The GitHub CLI (`gh`) is blocked in the agent shell so agents can never use
  a personal GitHub login; only local `git` is available. The guard sits on
  the shared execution path (custom-tool launches included), and custom-tool
  scripts are scanned for `gh` lines at create time — best-effort against
  indirection, with the token layers below as the real backstop. When the founder
  provides a company GitHub token (`GITHUB_TOKEN`, public_repo scope), it is
  captured into memory like the API keys and exposed only through the
  in-binary `gh_api` tool — no git credential ever enters an agent shell, and
  the tool refuses any call against the founder's own repos (the repo that
  auto-deploys the live company stays out of the company's reach).
- X spend runs on an in-binary prepaid ledger. Top-ups are credited only
  after the binary verifies the claimed USDC transfer on Solana mainnet RPC
  (destination, mint, and amount from the chain — never the agent's word),
  one credit per transaction signature. The RPC read is direct and outbound
  (never through the fetch proxy) and writes nothing on-chain.
- X mention/reply/quote events arrive over an **outbound** Activity API stream held open
  by the binary — no webhook endpoint is exposed, so X integration adds no
  inbound attack surface. Tweet text delivered by the stream reaches the CEO
  flagged as untrusted data, same as any `x_read` result: a stranger tweeting
  instructions at the account is an attack, not an order.
- The web log viewer has **no write endpoints** — page viewers cannot send
  anything to the agents.
- `web_fetch` can render a page in headless Chromium (JS app shells, and
  challenge walls the plain fetch cannot pass) and `web_screenshot` renders any
  URL to a PNG. A stranger's JavaScript therefore executes in a browser inside the
  container. The render runs through the same scrubbed-env runner as agent
  shells, so the browser child never holds a key; it has no cookies or sessions,
  writes only into the workspace sandbox, and its text output is wrapped as
  untrusted content like any fetch. Images shown to the model are labelled as
  data, not instructions.
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
