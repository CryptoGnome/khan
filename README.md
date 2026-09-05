<p align="center"><img src="assets/banner.png" alt="khan — an autonomous AI agent company" width="100%"></p>

<h1 align="center">khan</h1>

<p align="center"><b>An autonomous AI company in a single Rust binary — a CEO agent that hires, delegates, self-evolves, and never stops.</b></p>

<p align="center">
  <a href="https://khanbot.fun"><img src="https://img.shields.io/badge/watch_it_live-khanbot.fun-9ece6a.svg" alt="Watch it live"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-d4a017.svg" alt="License: MIT"></a>
  <img src="https://img.shields.io/badge/Rust-2021-orange.svg?logo=rust" alt="Rust 2021">
  <a href="https://railway.com?referralCode=SCj9lN"><img src="https://img.shields.io/badge/Deploy-Railway-blueviolet.svg?logo=railway" alt="Deploy on Railway"></a>
  <a href="CONTRIBUTING.md"><img src="https://img.shields.io/badge/PRs-welcome-brightgreen.svg" alt="PRs welcome"></a>
</p>

<p align="center">
  <b>🔴 Live instance: <a href="https://khanbot.fun">khanbot.fun</a></b><br>
  <sub>A khan running unattended with one directive — grow a crypto treasury. Everything it does streams to that page in real time.</sub>
</p>

Give khan one base directive and it works on it forever: the CEO agent hires
specialist employees, delegates tasks in parallel, rates the results, picks
models by price and intelligence, remembers what it learns, and rewrites its
own prompts, tools, and skills as it goes. You watch it all happen on a live
web log — and steer it with a single message, without ever stopping it.

Works with any OpenAI-compatible API ([bu0y](https://bu0y.com/), OpenRouter,
local servers — anything with `/v1/chat/completions`). Designed to be deployed
on [Railway](https://railway.com?referralCode=SCj9lN) and left running 24/7.

**Contents:** [Highlights](#highlights) ·
[Requirements](#requirements) ·
[Deploy on Railway](#deploy-on-railway-the-intended-way-to-run-khan) ·
[Live log viewer](#live-log-viewer) ·
[Run locally](#run-locally-for-development) ·
[How it works](#how-it-works) ·
[Contributing](#contributing--security--license)

## Highlights

- **Never idles, never spins** — an event-driven CEO kernel: episodes open on reports, alerts, and founder messages, close with a durable handoff note, and a heartbeat keeps strategy alive when the board is quiet.
- **Builds its own company** — hires/fires employee agents, each on the model its task deserves; objectives have owners, and worker reports route to the manager who owns the lane. Managers direct rather than do: every manager task opens with the live crew roster (busy/idle), serial hands-on grinding draws a crew-check past a soft line, and one worker can never be run by two callers at once. The CEO's board view carries a computed idle-capacity line (objectives active vs workers busy vs idle) so an episode can't close on "everything is owned" while the roster sits idle.
- **Picks its own seat** — the binary (never the model) chooses the CEO's model from a quality-ordered ladder against live marketplace prices, benches failures, drops to a cheap seat for quiet heartbeats, and watches the provider's balance so the tank refills before calls bounce.
- **Self-evolving** — prompts, custom tools, and skills are versioned in SQLite; the CEO improves them from outcome ratings and can roll back bad changes. Scheduled routines run mechanical checks at zero model cost, and review routines dispatch agents on a cadence for judgment work (site audits, adversarial code review).
- **Live and steerable** — real-time color-coded web log viewer; redirect the whole company with `khan tell "..."`, a message to its Telegram bot, or by editing one env var.
- **Makes real images** — a `generate_image` tool renders coin art and site imagery through OpenRouter image models for about a cent each, with the key held in the binary and spent on nothing else.
- **Survives everything** — state lives in `khan.db`; restarts and redeploys resume mid-mission.
- **Security-conscious** — immutable prompt rules, secret-scrubbed shells, injection-hardened web content, read-only public page. See [SECURITY.md](SECURITY.md).

## Requirements

Before first run you need:

1. **A model API key** (at least one):
   - `BU0Y_API_KEY` — bu0y key (fund with USDC, mint key at bu0y.com). Cheapest paid routing.
   - `OPENROUTER_API_KEY` — OpenRouter key, used for `:free` models and the
     `generate_image` tool (`image_model` in `khan.toml`).
   - Or any custom OpenAI-compatible endpoint (add a `[[providers]]` block in `khan.toml`).
   - Tip: OpenRouter exposes `openrouter/free`, which auto-routes to whatever
     free model is currently up — a convenient catch-all for easy work and
     failover. (It may route to a model without tool-calling, so keep at least
     one known tool-capable free model in your list for agent work.)
2. **A base directive** — the big goal the company should pursue.

## Deploy on Railway (the intended way to run khan)

khan is built to live on [Railway](https://railway.com?referralCode=SCj9lN) as a 24/7 worker: the
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
   - `TELEGRAM_BOT_TOKEN` + `TELEGRAM_CHAT_ID` *(optional)* — a direct line
     between founder and CEO. Messages from that one chat id land exactly like
     `khan tell`; the CEO replies (and proactively pings you) with its
     `message_founder` tool. The conversation is remembered: recent exchanges
     ride into every founder episode verbatim, and older ones are compacted
     into a long-term brief that keeps only what stays necessary. Any other
     chat that finds the bot is dropped and logged. Unset = the feature
     doesn't exist.
   - `X_CLIENT_ID` + `X_CLIENT_SECRET` + `X_REFRESH_TOKEN` *(optional)* —
     OAuth 2.0 credentials from your own X developer-portal app (confidential
     client, scopes `tweet.read tweet.write users.read offline.access`; the
     refresh token comes from a one-time PKCE authorization of the account).
     With all three set, every agent gets `x_post` and `x_read` (mentions /
     recent search) tools that act as the account via the official API;
     credentials are captured into memory at
     load and never reach an agent shell, and the rotating refresh token is
     persisted in the database after first use (the env value is a one-time
     seed). The binary also holds an X Activity API stream open in the
     background: mention, reply and quote events push in and wake the CEO as
     routine alerts (billed per delivered event) instead of paid mention
     polling — replies to the company's own posts are the bulk of real
     engagement and the reply wall permits answering them; obvious dm-me /
     pump-it bait is flagged on the alert so silence is the default. Each tweet
     gets one reply for good: the target is recorded permanently and `x_post`
     refuses a second reply to the same id, since the per-day billing ledger
     resets at UTC midnight and was letting yesterday's mention be answered
     twice. The stream
     is outbound-only — no webhook endpoint is exposed — and degrades to
     hourly reconnect attempts if the plan doesn't include the Activity API.
     The stream authenticates app-only: set `X_BEARER_TOKEN` (the app's
     bearer token from Keys & Tokens) or leave it unset and the binary mints
     one via the client-credentials grant.
     Unset = the tools and the stream don't exist.
     X spend is governed by an in-binary **budget ledger** (seeded at $5):
     every post, read, and delivered stream event debits it, paid calls
     refuse at $0, and the balance rides every tool result. The ledger
     mirrors X's real billing rules: per-resource charges deduplicated per
     UTC day (same-day re-reads are free), empty results free, owned reads
     at the reduced rate. Agents check the balance with `x_read` mode
     `budget` (free) — never via the X API or console.
   - `CC_FUND_SOL_ADDRESS` *(optional)* — the Solana address that recharges
     the X pay-per-use wallet: agents top up by sending USDC (SPL, mainnet)
     to it, then call `x_topup(tx_signature)`; the binary verifies the
     transfer on-chain (via `SOLANA_RPC` when set, else the public mainnet
     RPC) and credits the ledger with the
     verified amount, once per transaction. Unset = top-ups are refused with
     an alert-the-founder message.
   - `GITHUB_TOKEN` *(optional)* — a personal access token (public_repo
     scope) for the company's own GitHub account. Set = every agent gets a
     `gh_api` tool for the GitHub REST API (create repos, commit files, fork,
     PRs, issues) with the token held in memory, never in an agent shell; the
     tool refuses calls against the founder's own repos. Unset = the tool
     doesn't exist.
   - `FETCH_PROXY` *(optional)* — a residential proxy URL
     (`http://user:pass@gateway:port`) for web fetching. Datacenter IPs are
     walled off from much of the web (search engines, CDNs, many sites);
     with this set, `web_fetch`/`web_search` automatically retry blocked
     requests through the proxy, and agents can opt into it for scraping.
     The value is registered with the log redactor and never routes RPC or
     model-API traffic.
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
  wakes on it immediately. With the Telegram line configured, texting the bot
  from your phone does the same thing, and the CEO texts back. The binary
  also texts you unprompted when it detects itself crash-looping (three
  startups inside 15 minutes) — no model in that loop.
- **Stop the spend:** stop/remove the service. khan has **no built-in spend
  cap** (by design) — a cloud worker keeps calling the API until you stop it.

## Live log viewer

**See it in action: [khanbot.fun](https://khanbot.fun)** — a live khan instance,
streaming as you read.

While running, khan serves a web log viewer on `PORT` (default 8080 —
`http://localhost:8080` locally, your service URL on Railway). It streams the
activity log in real time — every event translated to a plain-English line and
color-coded per agent and per event type (chat, reports, milestones, team
changes, tool calls, errors) — with text filtering, per-agent toggles, and
click-to-expand raw detail on any row. It replays the last 300 events on
connect and reconnects automatically.

The page itself belongs to the company: it's served from
`workspace/viewer.html` (seeded from the built-in design on first boot), and
agents may redesign it however they like — try `khan tell "make the log page
look like a Bloomberg terminal"`. It can only ever be a display, though: the
server has no write endpoints, and an immutable security rule forbids agents
from giving viewers any way to send input to the company. If an agent ships a
broken page, delete `workspace/viewer.html` and the built-in design is served
again immediately.

The same server also gives every project its own site: point a wildcard DNS
record (`*.your-domain`) at the service, and a request for
`<name>.your-domain` serves static files from `workspace/sites/<name>/`
(index.html default). Agents put up a site per project by writing a folder —
no founder step per project; apex and www stay the company's own page.

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

- **CEO kernel** — one agent with the base directive plus control tools
  (`hire`, `delegate`, `fire`, `objectives`, `update_prompt`, `save_playbook`,
  `finish_episode`, …), run as an event-driven loop: an episode opens when
  something happens (a worker report, a routine alert, a founder message),
  closes with a `finish_episode` note that hands context to the next episode,
  and blocks on events in between instead of polling. A heartbeat opens a
  strategy episode when nothing has happened for `heartbeat_secs`.
- **Objectives board** — a ranked board with owners, plans, blockers, and
  status. Worker reports route to the manager who owns the objective; a plan
  untouched for a day while work advanced gets flagged, and a pivot closes its
  objective for a fresh one instead of mutating the old plan in place. Every
  objective carries a portfolio category (profit / growth / infra / explore),
  and once a week the reflection widens into a portfolio review: each category
  judged by its own yardstick (profit in dollars vs cost, growth in cost per
  attention, infra in reliability, exploration in learning per capped dollar),
  with each lane's measured share of the company's recent attention — so a
  social presence is never killed for earning nothing, and "it's marketing"
  never excuses unlimited spend.
- **Peer seats priced by what was paid** — every fill records the charge the
  gateway settled it at, and interchangeable models (`model_peers`) are
  compared on that realized price, not the catalog average: bu0y fills at the
  cheapest source, so the average can read a model at 1.5x its peer while the
  best ask sits at a twentieth. On the way into a dispatch an agent runs on
  the peer that has been more than `peer_switch_pct` cheaper over the last
  three hours, one dispatch in ten samples the other so a repricing gets
  noticed, and each move is logged once as `peer-seat`. A peer answering
  under 80% of its recent calls is never moved onto, however cheap, and an
  agent that has shown its model a picture in the last day is never moved at
  all — a per-token price says nothing about whether a peer's sources accept
  image content. Per-model
  price ceilings (`model_caps`) ride bu0y requests as `max_input_per_1m` /
  `max_output_per_1m`, so a fill waits for the cheap band (a 503 the retry
  loop sits out) instead of overflowing to a seller at four times the price.
- **CEO seat ladder** — the binary, never the model, picks the CEO's seat:
  the first model in a quality-ordered list that isn't benched by a recent
  failure and whose live marketplace price fits configured ceilings. Three
  stalls in ten minutes (a fill cut mid-answer, or one that burned two minutes
  to fail) bench a model for fifteen — including the ladder's own first rung,
  which no other rule could reach — and the ladder drops to the next seat;
  with every rung benched the floor seat still answers, so the company never
  stops thinking. Any agent's stalled call counts as evidence. Quiet
  heartbeats (nothing queued) run on a cheap `heartbeat_model`, escalating
  the moment real work drains in. The binary also polls the provider's
  balance; below a floor it alerts the CEO with a sized top-up target and
  benches it to the cheap floor seat until the tank is refilled — the strong
  model is earned back by topping up, not by arguing with the alert. The
  same arithmetic works the other way: while the tank is above the refill
  target (floor + three days of measured burn, a day-long average that
  survives restarts), the shell and custom-tool
  paths refuse any send to the provider's deposit address, so no
  self-invented runway doctrine can top up a tank that has days left. If calls
  bounce anyway (402), the seat goes into fuel
  emergency: the cheap floor model first, a free model if even that bounces —
  the company limps but never stalls, and runs its own top-up to recover.
- **Employees** — hired freely by the CEO, each with a role prompt and its own
  model (the CEO is told which models are free vs paid; an optional
  `model_policy` in khan.toml injects the founder's standing seat policy into
  every episode's brief, so it survives restarts instead of living in nudges,
  and `seat_denylist` enforces the part of it that matters: a denied slug is
  refused at `hire` and any agent already sitting on one is re-homed to the
  floor seat at its next dispatch, while the model stays usable as the
  automatic failure fallback). `delegate` runs one
  employee to completion; `delegate_parallel` runs several concurrently and
  returns all their reports; neither may be pointed at a manager by the CEO,
  whose episode is the only thing draining the company's queues. An employee that hits its iteration cap gets one
  final turn to file its report (finish is the only tool offered) before the
  kernel falls back to synthesizing one from the transcript tail. The CEO
  rates each report (`rate_work`, 1-5);
  per-agent/per-prompt-version stats feed the reflection step so prompt
  changes are judged on outcomes, not vibes.
- **Quiet heartbeats are cheap** — a heartbeat with nothing queued and a
  team already at work gets
  `quiet_heartbeat_max_steps` (2) instead of the full episode cap and no
  reflection payload, and one that puts no one to work doubles the wait to
  the next (`heartbeat_backoff_max_secs`); any event resets it. Measured
  before: 151 heartbeats a day, 88 dispatching nothing, 786 steps.
- **Dispatches are accounted** — every `dispatch` names the objective it
  advances (0 = upkeep; a task tagged 0 whose text names an objective is
  refused) and is classified build / check by its leading verbs.
  Three check-class dispatches in a row on one objective with nothing built
  between refuse the fourth; the same task shape three times in 24h refuses
  the fourth with the add_routine redirect. On an `explore` objective only a
  task that names the revenue idea it advances (`id65`, `row 54`) counts as a
  build, so another scan cycle cannot pass for conversion. The board shows each
  objective's 24h build/check mix and flags ALL CHECKS and explore objectives
  with no build as CONVERT OR KILL. `scripts/throughput_audit.py` measures all
  of it.
- **Lanes answer for their own times** — every objective carries a review
  time at hour resolution (`2026-09-03T15:00Z`) and the criterion that would
  kill it. Every board line carries its time; a lane past it reads REVIEW DUE,
  and one set further than `max_review_horizon_hours` ahead reads BEYOND THE
  HORIZON — a shelf, not a commitment. Both stand in the CEO brief as a
  decision owed this episode: close, drop, or recommit. A push made while the
  old time is landing (inside its last six hours) must say what changed, and a
  lane may be recommitted only `max_recommits` times without a build-class
  report rated in between — after that only done or drop remain. A new
  objective is refused once `max_active_objectives` are open, whatever their
  dates say.
- **An episode is bounded by the clock, not just by steps** — a CEO episode
  ends at `episode_max_minutes` however few steps it has taken, blocking runs
  (`delegate`) are rationed per episode by `max_blocking_delegates`, and routine
  alerts drain *during* an episode rather than only when the next one is
  composed. An episode is the only loop that reads alerts, reports and founder
  messages: on 2026-09-03 thirteen serial delegates held one open for four
  hours, 153 alerts queued behind it, and a launch blocked on a budget question
  went unread for three of them.
- **A launch fires in the window and is not done until it is posted** — the
  shell refuses a live token launch outside `launch_window_open_utc`–
  `launch_window_close_utc` (default 13:00–03:00 UTC, 9am–11pm Eastern), the
  CEO brief says when the window opens or closes, and every launch booked in
  the last 48 hours that no `x_post` has named (ticker or mint) stands in the
  brief until one does. Five of eight launches on 2026-09-04/05 fired between
  01:23Z and 08:15Z into an asleep audience, and the X lane's own session plan
  never mentioned them. Custom-tool descriptions are capped at 280 characters
  (`create_tool` refuses longer; older rows ride their first paragraph) — the
  catalogue rides every call of every agent, and description text alone was
  32k of its 48k characters.
- **The board carries what the binary knows** — each profit lane's line shows
  the ledger's own tally (net per asset over every `pnl` row tagged with the
  objective, plus the closed trades on every ticker those rows name),
  computed from workspace.db rather than written by the CEO. The
  trend-launch lane ran eight launches at an identical loss and stayed open
  because no line ever showed the number.
- **Ops lanes are run by their routines** — an objective of kind `ops`
  (treasury checks, listings, inbox, X) shows its owner's routine status on the
  board (`7/7 ok`, or which are failing), and a dispatch on it is refused while
  every routine reports ok, except one per 24h as the human look. On boot every
  shell routine fires within the minute, so restart triage is the scripts' job
  rather than twenty minutes of the CEO's.
- **A rating of 4 or 5 needs an artifact** — the rated agent's report must name
  something that exists: a file under the workspace, a transaction hash, a
  signature. 477 of 594 ratings in one week were 5s, NOOPs included, so ratings
  drove nothing.
- **The skill index is what is in use** — every call carried all 243 skills
  (about 5k tokens, 11,800 times a day) while 60 had been loaded that week.
  The index now lists skills loaded in the last 14 days or created in the
  last 3; `use_skill` with a partial name finds the rest.
- **A prompt is text, never a pointer** — `update_prompt` refuses a bare URL or
  anything under 400 characters, and a stored version that fails that check is
  skipped at read so the last real prompt stays live. The CEO ran five days on
  its mandate alone after saving a URL as version 11 of its own prompt.
- **Ideas answer for their own dates** — every `revenue_ideas` row carries a
  review date, and the CEO brief stands a list of the rows whose date has
  passed while they are still premise, candidate, screening, watch or
  verified-open. Each one is a decision owed: hand it to a lane, kill it with
  the number, or name the missing fact and its date. Scanning ran on a routine
  and converting ran on nobody's calendar, so 16 premise rows and 13 candidates
  banked up and the only writes they got were appended notes.
- **The log bounds itself** — the site's stats daemon uses `run_log` as the
  bus to the viewer's event stream at ~80 KB every 12 seconds; the binary
  ages ticker rows out after six hours on the same path that writes them,
  so the volume cannot fill the way it did on 2026-09-01.
- **Live steering** — `khan tell "..."` from a second terminal (or a Telegram
  message from the allowlisted founder chat) queues a founder message; the
  running CEO wakes on it immediately. No restart needed. A `khan tell` is
  also a standing directive: it stays in the CEO's brief every episode until
  the CEO acknowledges it done with `ack_founder`, so an instruction that
  misses one episode's step cap is not lost.
- **Routines have owners** — a routine can name the employee who owns its
  domain, and its alerts then dispatch straight to that owner (report
  routing brings back the outcome); only ownerless alerts wake the CEO,
  which is the signal to assign or hire an owner.
- **Routines** — the CEO schedules its own recurring checks: shell routines
  run inside the binary at zero model cost and only surface deviations
  (nonzero exit or `ALERT` output), while review routines dispatch an agent on
  a cadence for work that needs judgment — an outsider-eyes site review, an
  adversarial audit of the scripts.
- **Tool schemas are checked before they can break every call** — an agent's
  custom tool is refused at creation if its parameter schema is not valid JSON
  Schema (Python type names like `str`, a `required` object instead of an array
  of names, requiredness on the property itself), and a malformed schema already
  in the store is dropped to a permissive one rather than sent. The tool list
  rides every request, so one bad schema is refused by strict sellers and fails
  the whole fleet over to dearer routes.
- **The live database is not the company's to move** — a shell command that
  would rename, copy, overwrite, truncate or remove `khan.db` is refused
  (reads and `VACUUM INTO` a differently named copy pass), and the binary
  checks on every fuel poll that the file at its database path is still the
  one it opened; if not it exits for a restart rather than run blind. On
  2026-09-03 an agent swapped a vacuumed copy into place and the running
  binary spent seven hours writing to a file nobody could see. The X refresh
  token that rotated inside that file was lost with it, so a refresh refused
  as `invalid_request` now falls back to a fresh `X_REFRESH_TOKEN` seed when
  one is set, and the stream backs off to hourly instead of retrying.
- **The volume is a tank too** — free space is checked on the fuel poll and a
  `disk-low` alert wakes the CEO below 512 MB, naming what to cut. Oversized
  tool output past 16 MB is cut rather than spilled (nobody reads a 900 MB
  file back), and the spill directory is bounded at 256 MB, largest first.
- **Model failover** — if an agent's model keeps failing (free-tier 429s/outages),
  the call is answered by the next available free model automatically and the
  swap is logged. A refusal that names a ceiling that would fit
  (`error.retry_max_tokens`, on a 400 as well as a 503) is re-sent at
  exactly that number rather than abandoned, and followed down while the
  gateway keeps naming a smaller one, and a stream that breaks after
  delivering output keeps what arrived — those tokens are billed, so a retry
  would buy the same answer twice — while dropping any tool call whose
  arguments were cut off mid-JSON. A break before any output is free and is
  retried. That named ceiling is derived from the model's recent speed, so a
  degraded route can hand back one too small to answer inside: when a call
  truncates on a ceiling the gateway chose, the fallback ladder tries another
  model (which is quoted its own) and the route takes a stall strike, while a
  budget the binary chose and the model blew still fails outright. A summary that spends its whole
  budget reasoning gets one wider attempt before the history is left to grow.
  Summarising calls ask for a summary-sized ceiling rather than the model's whole 64k,
  since the gateway reserves against whatever is asked. The client's own deadline sits above the gateway's 480s fill
  ceiling, since hanging up first pays for output nobody reads. Every bu0y
  request carries a decode-speed floor (`min_tokens_per_sec` in the provider
  block) so the cheapest route is skipped when it is one that cannot finish;
  when no route clears it the gateway's `unmet_speed` refusal goes straight
  to the fallback ladder and counts as a stall strike, so a floored seat is
  benched off the CEO ladder rather than refused again every episode, and a fill on an unmeasured route is logged as
  `speed_floor=unverified`.
- **Work tools** (all agents): file read/write/list (confined to `workspace/`),
  shell (with local `git` for version control; the GitHub CLI is blocked so
  agents can never reach your GitHub login), web fetch + DuckDuckGo search (JS app shells render in headless Chromium automatically, with the page's declared dates and its CSS/JS bundle URLs listed), `web_screenshot` and `view_image` so a vision seat actually sees pages and art,
  SQL against a scratch `workspace.db` (the tool's description carries the
  live table list, and a wrong name gets the real schema back in the error),
  `generate_image` (real renders via
  OpenRouter image models, ~$0.01 each, the key never enters an agent shell),
  and `remember`/`recall` memory. Oversized tool output isn't dropped: the
  full text is saved to `workspace/.spill/` and the truncation marker names
  the file, so an agent reads the rest back instead of re-running the command.
  Shell-style output keeps its tail visible (errors land at the end); web and
  file content keeps its head (the untrusted-content marker leads). Spill
  files self-purge after a week, swept on each new spill.
- **Memory** — SQLite FTS5. Relevant memories are auto-injected into context;
  recall also scans skill bodies and surfaces matching excerpts, so a fact
  recorded in a skill contradicts a false claim wherever that claim travels;
  long histories are compacted into summaries by a cheap model.
- **Custom tools** — any agent can call `create_tool` to turn a Python or
  PowerShell script into a real, schema-described tool that every agent can
  call from then on (the script reads its JSON args from the `KHAN_TOOL_ARGS`
  env var and prints its result; scripts that invoke the blocked GitHub CLI
  are refused at create time). Tools are versioned in `khan.db` like
  prompts: `create_tool` with the same name makes a new version,
  `rollback_tool` reverts a bad one. The CEO can also delegate tool-building
  to an employee.
- **Skills** — Claude-style reusable how-to documents the agents author
  themselves with `create_skill` (markdown: steps, gotchas, checklists). Every
  agent sees a compact skill index each turn and loads a skill's full
  instructions with `use_skill` before doing work it covers. Versioned like
  tools: same name = new version, `rollback_skill` reverts, `retire_skill`
  deletes one that no longer earns its index line. Skills are judged on
  outcomes like prompts: every `use_skill` load is joined to the loader's
  next work rating, and reflection surfaces the worst-scoring and long-unused
  skills. Curated skills can also be seeded from the repo's `skills/`
  directory (one `.md` per skill, first line the description): seeds land at
  boot, a changed seed file ships as a new version while the skill is still
  seed-origin, and a skill the company has since evolved is never overridden.
  Skills are written portable (method and why in the body, instance facts in
  one marked section, no secrets) so a live company's best skills can be
  harvested back into `skills/` as better seeds for fresh installs.
- **Tool health** — every tool call's outcome is recorded. Failures show up
  immediately in the activity log (red), and repeated failures are aggregated
  into the reflection step as "N of M recent calls FAILED — last error: …", so
  the CEO sees broken infrastructure as a *pattern* and routes around it
  (diagnose → build a replacement with `create_tool` → save the workaround as
  a skill) instead of silently retrying a dead tool forever.
- **The CEO directs, it does not do** — `write_file`/`create_tool` are
  stripped from its schema, and its hands-on execution (shell, SQL, custom
  scripts) is rationed per episode: discovery is free, but past 4 calls each
  result carries an "is this CEO work or a delegation?" challenge, and past
  12 the tool refuses outright — that volume is a doom loop, not discovery.
  Reads, dispatching, and rating are unlimited; every rationed tool is
  callable by an employee, so sends and exits dispatch like any other work.
- **Self-evolution** — prompts live in `khan.db`, versioned. Reflection rides
  the heartbeat episodes: the CEO reviews the activity log and outcome
  ratings, may rewrite its own or employees' prompts (`update_prompt`), roll
  back bad changes, and save playbook lessons. Everything survives restarts,
  so the org genuinely improves across runs.
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

## Contributing · Security · License

- **Contributing:** PRs welcome — see [CONTRIBUTING.md](CONTRIBUTING.md) for
  the ground rules (keep it lean, match the style, don't weaken the security
  layers).
- **Security:** the threat model and private vulnerability reporting are in
  [SECURITY.md](SECURITY.md). Short version: khan is not a sandbox and the log
  viewer is unauthenticated — deploy it in a container and keep the URL
  private.
- **License:** [MIT](LICENSE).
