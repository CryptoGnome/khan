How to write a zero-model-cost ROUTINE script: ALERT/pass-fail semantics, exit codes, /proc scanning gotchas (self-exclusion, zombies), and the DB-derived-figures rule. Use when creating any recurring check.
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

## Gotchas
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
