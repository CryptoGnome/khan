# Kernel rework — 24h baseline (captured 2026-08-29T20:03Z)

Pre-change metrics the phase gates compare against.

| metric | 24h value |
|---|---|
| CEO events (non-thinking) | 7,732 (~322/h) |
| CEO spoken turns | 2,616 (~109/h) |
| Compactions | 149 (~6.2/h) |
| Compaction failures | 20 |
| Idle holds triggered | 38 |
| Employee silent-stops | 33 |
| Restarts | 37 |
| CEO dispatch/delegate calls | 288 |
| Model calls (top) | deepseekv4flash0731 4,105 (116 fail) · deepseekv4flash 1,922 (2) · glm53flash 229 (1) · claudesonnet5 205 (0) · grok46 101 (6) |

Targets after Phase 1: CEO turns/hour −50%, poll-signature runs extinct, report→action latency unchanged.
After Phase 2: CEO compactions = 0, cost/hour −40%, no duplicate dispatches, restart recovery < 1 min.
After Phase 3: CEO episodes/hour below Phase-2 soak with equal-or-higher dispatch throughput.
