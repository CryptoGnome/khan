# Kernel throughput — 24h baseline and the fixes (captured 2026-09-02T01:10Z)

The founder's read: "it did one launch today and it's really not building anything
new even though we've given it a lot of ideas. It seems like it goes in circles over
the same task instead of getting new ones." The numbers agree.

## Baseline (24h ending 2026-09-02T01:10Z)

| metric | 24h value |
|---|---|
| Shipped | 1 launch (PINKPROOF), killed at 2h45m −19.6%; 4 kill-exits; 1 listing submitted; 3 obj6 decisions logged as "deferred" |
| CEO episodes | 195 — 151 heartbeats (786 steps), 27 report, 13 founder, 3 event, 1 alert |
| Heartbeats that dispatched nothing | 88 of 151 |
| CEO episodes ending at the step cut-off | 56 (29%) |
| CEO model calls | 1,217 — more than its four busiest employees combined |
| CEO hands-on calls | shell 348, sql 201, khan_db_query 34 |
| CEO hands-on per episode | 0: 43 · 1-4: 104 · 5-12: 48 · 13+: 0 |
| Ratings | 269; 218 say the CEO re-verified the report itself before rating |
| Dispatches | 384 — 190 build-class, 188 check-class, 6 other; 173 untagged (no objective) |
| Re-dispatch / resume / re-run chains | 64 |
| Reports | 381 to the CEO, 35 routed to owners; 8 synthesized (cap/stop) |
| Employee calls per report | eng-mgr 21, ideation-1 19.6, floors-1 19.4, pons-mgr 18.4, scan-mgr 18.1, launch-mgr 16.6 |
| Revenue ideas | 1 done, 1 building, 13 candidate, 11 premise, 7 screening; ids 55–62 all at PREMISE |
| Skills | 61 (305 KB of bodies), 313 loads; the 61-line index rides every turn of every agent |
| Dispatch task size | median 1,085 chars, p90 1,996 |
| Model calls by model | glm53flash 6,738 · glm5 225 · deepseekv4flash 25 |
| Fuel burn | ~$16/day |

Top CEO shell purposes, verbatim shape: "spot-checking launch-mgr's claim",
"verifying x-mgr's claimed evidence file", "independently spot-check scan-mgr's
on-chain claims", "checking the freshest evidence files from the last cycle".

## Diagnosis

1. **Verification is the company's main product.** A model-written skill,
   `verify_the_world`, says: before rating 4 or 5, check the load-bearing claim
   against the world. It exists for a real reason (two episode notes tonight
   claimed a skill fold that never happened) but at company scale every report
   costs a second, CEO-priced pass, and half of all dispatches are checks of
   earlier work.
2. **Quiet heartbeats are the fuel sink.** Every 300 s the CEO wakes, reviews the
   log, spot-checks, and closes — 5 steps on average, nothing dispatched in 88 of
   151. That is ~750 CEO calls a day directing nothing. The reflection payload
   (log tail, ratings, skill and model tables, catalog, portfolio, capacity) is
   rebuilt and sent on every one of them.
3. **The same task shape recurs.** "metadata gate coin image for PINKPROOF" ×4,
   "sizing gate re-run" ×2, "mid-cycle sweep cycle N" — work a routine script
   does for free, dispatched as agent runs. The pattern is documented in a skill
   (`routine_script_pattern`, loaded 9 times) and not chosen.
4. **Ideas do not graduate.** The scan lane produces premise-stage ideas faster
   than anything executes them; nothing on the board makes an explore objective
   convert.
5. **Half of dispatches carry no objective**, so the board's in-flight counts,
   report routing to owners, and every per-objective signal miss them.
6. **The prompt argues both sides.** Rule 4: "if every task in flight is a check,
   a fix, or a verification, you have drifted." Opening line: "if a goal seems
   complete, verify it." Prose, both; the drift side lost.

## Fixes (all in the binary, each with a test)

| # | fix | mechanism | measured target |
|---|---|---|---|
| F1 | Quiet heartbeats are cheap | A heartbeat with nothing queued and no finished task gets `quiet_heartbeat_max_steps` (2) instead of `episode_max_steps`, and no reflection payload. A quiet heartbeat that dispatches nothing doubles the next heartbeat interval, up to `heartbeat_backoff_max_secs` (1800); any event resets it. | heartbeat steps/day 786 → under 250; CEO calls/day 1,217 → under 600 |
| F2 | Checks are budgeted per objective | Every dispatch/delegate is classified build / check / other by its leading verbs and recorded. Three consecutive check-class dispatches on one objective with nothing built between refuse the fourth: dispatch progress, write a routine, or rate on the evidence in the report. | check-class share 49% → under 30% |
| F3 | Repeats become routines | The same task shape (objective + first six words, folded) dispatched three times in 24h refuses the fourth with the add_routine redirect. | repeat shapes ≥3: 2 → 0; re-dispatch chains 64 → under 20 |
| F4 | The board shows the mix | Each objective's line carries its 24h build/check counts; an objective that is all checks is flagged CONVERT OR KILL. Explore objectives (kind explore) that have not produced a build-class dispatch in 24h are flagged the same way. | ideas moving out of premise/candidate: 0/day → ≥1/day |
| F5 | Every dispatch names its objective | `objective` is required on dispatch; 0 means company upkeep and is accounted like any other. | untagged 173 → 0; reports routed to owners 35 → majority |
| F6 | Hands-on budget tightened | CEO_EXEC_SOFT 4 → 3, CEO_EXEC_HARD 12 → 8. Today's 5-12 band was spot-checks, not investigation. | hands-on per day 583 → under 250 |
| F7 | Verification doctrine narrowed | `verify_the_world` seed: check the world only for money-moving, public-facing, or irreversible claims; everything else is rated on the evidence the report carries (txid, row id, file hash). Live skill via directive (stands until acked). | ratings that self-verify 218/269 → under 80 |

Not done now, noted: the 61-line skill index on every turn (F8), dispatch task
bloat (F9), employee calls-per-report ~18 on the managers (F10). Re-measure after
F1–F7 land; the second two may fall on their own.

## How to re-measure

The queries behind the table live in `scripts/throughput_audit.py`; run it over
SSH against `/data/khan.db` and `/data/workspace/workspace.db` for any 24h
window. Gates: every target above, read from the same script, 24h after deploy.
