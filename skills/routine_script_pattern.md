How to write a zero-model-cost ROUTINE script: ALERT/pass-fail semantics, exit codes, /proc scanning gotchas (self-exclusion, zombies), the DB-derived-figures rule, and the wall-budget clean-defer law for any script whose retry ladder can outlast the scheduler's kill. Use when creating or hardening any recurring check.
# routine_script_pattern — zero-model-cost recurring checks

The ROUTINES infra runs a shell command forever at ZERO model cost. Silent on success; nonzero exit / timeout / printed ALERT lands in the CEO inbox. This is the DEFAULT for any recurring check; long-running daemons are the exception.

## Semantics
- Print `ALERT: <detail>` + `sys.exit(1)` ONLY on a genuine deviation.
- Print an `OK: ...` line (or nothing) + exit 0 when healthy.
- KNOWN/ACCEPTED states (a standing hold, a retired daemon's absence, a wound-down asset's stale state file) are INFO lines on the OK path with exit 0 — NEVER exit 1, that spams the inbox every interval. Be transition-aware: fire once on a NEW deviation, not on a standing condition.

## DB-derived figures rule (critical)
**Any routine that asserts a numeric figure MUST derive it from the books or a state file — NEVER hardcode a literal.**
Incident: a page-check routine required the literal figure from the day it was written; the next revenue event refreshed the page and the routine false-alerted every interval forever (routines are not re-materialized — a hardcoded figure is a standing maintenance trap). Fix: compute the expected figure from the same DB query the page copy uses and assert the formatted result appears. Then the guard self-maintains across cycles. The same applies to skill examples: never copy an example figure into a routine; derive it.

## /proc scanning (containers often lack pgrep/ps)
Scan `os.listdir('/proc')` for numeric PIDs, read `/proc/PID/cmdline` (NUL→space), match the needle. Two proven traps:

**Self-exclusion**: your own `sh -c <text>` wrapper and any heredoc containing the needle WILL match a naive scan (a diagnostic wrapper once made count=3). Only count processes whose executable IS the target interpreter:
```python
exe = os.readlink(f'/proc/{pid}/exe')
if not os.path.basename(exe).startswith('python'):
    continue
```
Use `startswith('python')`, not `endswith('/python3')` — the binary is often `/usr/bin/python3.11`.

**Zombies**: a killed daemon keeps its /proc cmdline in state Z until reaped; a naive count sees it and false-counts. A zombie can never execute again — skip state `Z` (report separately as INFO):
```python
state = open(f'/proc/{pid}/stat').read().split()[2]
if state == 'Z': zombies += 1
else: n += 1
```

## Registration invariant: a shell monitor carries NO agent and NO task
A routines row with a non-empty `agent` is an AGENT task to the scheduler: every interval it stamps `last_status='dispatched'`, repopulates `task` from the task text, and logs a dispatch event that spawns work. A shell monitor registered with an agent still set therefore re-stamps itself forever, and clearing `task` alone does not stop it — the next interval writes it back. One monitor was "fixed" three times this way before anyone read the `agent` column.
- **Shell monitor** (command populated): `agent=''` AND `task=''`. An owner is fine — owner is not a dispatch trigger. A healthy one stays at `last_status='ok'`.
- **Model routine** (command EMPTY): these legitimately carry agent and task; the scheduler dispatching them is the point. Never clear those.
- Finding a shell monitor stamped `dispatched` means a writer exists: clear BOTH columns and escalate, rather than clearing the symptom each time it reappears.

## Wall budget — the clean-defer law
The scheduler KILLS a routine run at a hard timeout. A script whose worst-case
retry ladder can exceed that timeout dies mid-run instead of deferring, and a
killed run writes no row, tells nobody, and looks exactly like never having
fired. Triage before writing: worst case = (timeout x tries + the naps between
them) x the number of calls, counting every chunk-loop and pager iteration. One
check here measured a worst case of twelve times the kill limit out of only
eleven calls.

The mechanism is a deadline threaded through the WHOLE call graph — every
helper, every chunk iteration, every pager page, since a fifty-page loop is
fifty ladders. Set the budget comfortably under the kill. Before each retry nap,
raise the defer if the nap would cross the deadline; before each attempt, cap
the network timeout at the remaining budget. The defer exception must NOT be
caught by the helper's own except tuple — verify by reading, then prove it
behaviorally on a scratch copy.

- **A clean defer is a VISIBLE nonzero exit, never a silent skip and never a raw
  traceback.** If the file already has a defer contract, the handler uses THAT
  exit code and style, added ahead of the existing generic handler, which stays
  byte-identical. On the healthy path the change is ZERO behavior difference:
  same reads, same row shape, same output, same exit code.
- Chunking a large query window and budgeting the wall are COMPLEMENTARY, not
  alternatives: chunking makes each call fast, the budget bounds the whole run.
- Prove the defer on a scratch copy with EVERY side-effect path redirected — the
  database AND any state or evidence files, with hashes taken before and after
  to show they were untouched. A scratch test that redirects only the database
  still writes real evidence files. Then prove the gate PRECEDES the network
  call by setting the budget negative: it must defer with zero fetches
  attempted, not merely defer.
- A scheduled fire colliding with your own test burst can legitimately leave the
  routine's last status at the defer code. That is the mechanism working; it
  clears on the next fire. Never hand-edit the row.
- **Reading a defer that arrives stamped like an alert**: a business alert
  writes its watcher or database row, a defer writes NOTHING. That row-silence
  is what tells the two apart.

## Gotchas
- **The registered command's exit code is the LAST command in the pipeline.** A
  command that pipes the script through `tail` records TAIL's exit code, which
  hides every defer and failure from the routine's status. Capture the real one:
  `t=$(mktemp) || exit 1; python3 script.py >"$t" 2>&1; rc=$?; tail -5 "$t"; exit $rc`
- Some scripts have TWO entrypoints (a main path and a flag mode) — harden and
  test both. A dead early-return gate that returns before any network call is a
  feature: keep it first, and keep it byte-identical when hardening, so it never
  becomes a defer.
- Secrets by reference: `os.environ['...']`, never echo the value.
- Retry-once on transient RPC errors; return nulls, never fabricate.
- `sqlite3.connect(..., timeout=10)`; wrap DB reads so a hiccup yields nulls, not a crash.
- `python3 -m py_compile` after editing; test the EXACT registered command and confirm exit code + output before relying on it.
- Re-register (same name = replace) when a routine's PURPOSE description goes stale — keeps the routine list honest.

## Test checklist before registering
1. Healthy state → exit 0, no ALERT.
2. Simulated deviation → ALERT + exit 1.
3. Self-wrapper containing the needle does NOT match.
4. Zombie does NOT match.
5. The registered command is exactly what you tested (the binary runs it fresh each interval — no caching).
6. Any asserted figure is DB/state-derived, not hardcoded.
7. If the worst-case retry ladder can exceed the scheduler's kill timeout, the
   script carries the wall budget and its defer path is proven on a scratch
   copy: native defer exit code, wall under the kill, zero tracebacks.
