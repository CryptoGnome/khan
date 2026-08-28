# khan

A lightweight autonomous agent orchestrator in Rust. One CEO agent gets a base
directive and works on it forever — hiring specialist employee agents, delegating
tasks, picking models per task (free models for easy work), remembering what it
learns, and evolving its own prompts over time.

Works with any OpenAI-compatible API: [bu0y](https://bu0y.com/), OpenRouter,
local servers, anything with a `/v1/chat/completions` endpoint. Designed to be
deployed on [Railway](https://railway.com) and left running 24/7 — see
[Deploy on Railway](#deploy-on-railway-the-intended-way-to-run-khan).

## Requirements

Before first run you need:

1. **A model API key** (at least one):
   - `BU0Y_API_KEY` — bu0y key (fund with USDC, mint key at bu0y.com). Cheapest paid routing.
   - `OPENROUTER_API_KEY` — OpenRouter key, used for `:free` models by default.
   - Or any custom OpenAI-compatible endpoint (add a `[[providers]]` block in `khan.toml`).
   - Tip: OpenRouter exposes `openrouter/free`, which auto-routes to whatever
     free model is currently up — a convenient catch-all for easy work and
     failover. (It may route to a model without tool-calling, so keep at least
     one known tool-capable free model in your list for agent work.)
2. **A base directive** — the big goal the company should pursue.

## Deploy on Railway (the intended way to run khan)

khan is built to live on [Railway](https://railway.com) as a 24/7 worker: the
`Dockerfile` has everything preinstalled (Rust build, `git`, `python3`),
a volume keeps the mission alive across redeploys, and the built-in web log
viewer gives you a live window into what the company is doing from anywhere.

1. **Fork/push this repo to GitHub.** Before you do, edit `khan.toml.example`
   (models) — that file is baked into the image as the default config.
2. **Create a Railway service from the repo** (Railway auto-detects the
   `Dockerfile`), or run `railway up` from this folder with the CLI.
3. **Add a Volume mounted at `/data`.** It holds `khan.db` and `workspace/`,
   so the company — team, memories, evolved prompts, work in progress —
   survives every redeploy and restart.
4. **Set service Variables:**
   - `OPENROUTER_API_KEY` and/or `BU0Y_API_KEY` (whatever your `khan.toml` providers need)
   - `KHAN_DIRECTIVE` — the base directive. This is where the big goal goes;
     multi-line/multi-paragraph values are fine in Railway's variable editor.
5. **Deploy.** The image's default command is `khan auto`: first boot starts
   the mission from `KHAN_DIRECTIVE`; every later deploy **resumes** it.
6. **Enable public networking** on the service to get the live log viewer at
   your service's URL (Railway injects `PORT` automatically). See
   [Live log viewer](#live-log-viewer). Note the viewer has no auth — anyone
   with the URL can read the logs.

### Steering a running deployment

- **Change the big goal:** edit the `KHAN_DIRECTIVE` variable. Railway
  redeploys on save; on boot khan adopts the new directive, keeps the whole
  company (team, memories, prompts), and delivers a founder message telling
  the CEO the directive changed.
- **Nudge without redeploying:** open a shell on the service (`railway ssh`)
  and run `khan tell "drop the pricing page, focus on the scraper"` — the CEO
  acts on it at its next iteration.
- **Stop the spend:** stop/remove the service. khan has **no built-in spend
  cap** (by design) — a cloud worker keeps calling the API until you stop it.

## Live log viewer

While running, khan serves a web log viewer on `PORT` (default 8080 —
`http://localhost:8080` locally, your service URL on Railway). It streams the
activity log in real time — every event translated to a plain-English line and
color-coded per agent and per event type (chat, reports, milestones, team
changes, tool calls, errors) — with text filtering, per-agent toggles, and
click-to-expand raw detail on any row. It replays the last 300 events on
connect and reconnects automatically.

## Run locally (for development)

```powershell
copy khan.toml.example khan.toml   # then edit models
$env:BU0Y_API_KEY = "bu0y_..."
$env:OPENROUTER_API_KEY = "sk-or-..."
cargo build --release

# start a new run (unbounded — runs until Ctrl+C)
.\target\release\khan.exe run "Build and maintain a website that tracks retro game prices"

# stop with Ctrl+C (state is saved), continue later:
.\target\release\khan.exe resume

# steer it WITHOUT stopping — from a second terminal:
.\target\release\khan.exe tell "drop the pricing page, focus on the scraper"
```

The terminal shows the same activity log. Every 5 iterations a cumulative
token count is printed. **There is no spend cap — watch it.**

## How it works

- **CEO loop** — one agent with the base directive plus control tools:
  `hire`, `delegate`, `fire`, `list_team`, `update_prompt`, `rollback_prompt`,
  `save_playbook`, `finish` (milestone report; work continues).
- **Employees** — hired freely by the CEO, each with a role prompt and its own
  model (the CEO is told which models are free vs paid). `delegate` runs one
  employee to completion; `delegate_parallel` runs several concurrently and
  returns all their reports. The CEO rates each report (`rate_work`, 1-5);
  per-agent/per-prompt-version stats feed the reflection step so prompt
  changes are judged on outcomes, not vibes.
- **Live steering** — `khan tell "..."` from a second terminal queues a founder
  message; the running CEO picks it up on its next iteration. No restart needed.
- **Model failover** — if an agent's model keeps failing (free-tier 429s/outages),
  the call is answered by the next available free model automatically and the
  swap is logged.
- **Work tools** (all agents): file read/write/list (confined to `workspace/`),
  shell (with local `git` for version control; the GitHub CLI is blocked so
  agents can never reach your GitHub login), web fetch + DuckDuckGo search,
  SQL against a scratch `workspace.db`, and `remember`/`recall` memory.
- **Memory** — SQLite FTS5. Relevant memories are auto-injected into context;
  long histories are compacted into summaries by a cheap model.
- **Custom tools** — any agent can call `create_tool` to turn a Python or
  PowerShell script into a real, schema-described tool that every agent can
  call from then on (the script reads its JSON args from the `KHAN_TOOL_ARGS`
  env var and prints its result). Tools are versioned in `khan.db` like
  prompts: `create_tool` with the same name makes a new version,
  `rollback_tool` reverts a bad one. The CEO can also delegate tool-building
  to an employee.
- **Skills** — Claude-style reusable how-to documents the agents author
  themselves with `create_skill` (markdown: steps, gotchas, checklists). Every
  agent sees a compact skill index each turn and loads a skill's full
  instructions with `use_skill` before doing work it covers. Versioned like
  tools: same name = new version, `rollback_skill` reverts.
- **Self-evolution** — prompts live in `khan.db`, versioned. Every
  `reflect_every` iterations the CEO reviews the activity log and may rewrite
  its own or employees' prompts (`update_prompt`), roll back bad changes, and
  save playbook lessons. Everything survives restarts, so the org genuinely
  improves across runs.
- **Security layers** — defense against prompt injection and secret leaks:
  an immutable security preamble is appended to every agent's system prompt
  from code (not the editable prompts table, so neither evolution nor an
  injection can remove it): tool output/web/file content is data-not-instructions,
  never reveal secrets, never download-and-execute, never weaken the rules.
  `shell` child processes get API keys/tokens/secrets stripped from their
  environment, and web content arrives wrapped in explicit untrusted-content
  markers. Note: `shell` is still real shell access on your machine — khan is
  not a sandbox; don't point it at directives you wouldn't trust a script with.
- **State** — `khan.db` holds agents, histories, prompts, memories, and the run
  log; `khan resume` picks up exactly where it stopped.

## Files

- `khan.toml` — config (providers, models)
- `khan.db` — internal state (do not hand-edit while running)
- `workspace/` — the agents' working directory, including their `workspace.db`
