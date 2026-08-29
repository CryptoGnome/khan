# Kernel rework — results vs baseline

Baseline window: the 24h before the rework (see kernel-baseline.md).
Measured live on the production deploy, 2026-08-29.

## Phase 0 — silent-stop reports
Employees that stop without reporting now hand back a synthesized report
built from their last transcript entries, marked PARTIAL. Baseline: 33
silent stops / 24h, each one a dead end the CEO had to re-derive. Since
the change: 0 unexplained "(employee stopped without a report)" lines.

## Phase 1 — event-driven loop
The free-running loop (baseline: 7,732 CEO events / 24h, most of them
polls) is gone. The CEO blocks between episodes and wakes on: a finished
dispatch, a founder message, a routine alert, or the 900s heartbeat.

- Measured founder-message wake latency: **19 ms** from `khan tell` to the
  CEO turn starting.
- "waiting — event-driven idle" entered cleanly; no polling turns while
  dispatched work ran.

## Phase 2 — episodic CEO
The resident transcript (baseline: 149 compactions / 24h, each a lossy
rewrite of the CEO's whole memory) is gone. Every episode composes its
context fresh from durable state — directive, roster, board, log tail,
last episode note — and closes with a note that is the only thing that
survives.

- One-way migration ran on first boot: legacy transcript distilled into
  episode #1's note, then never read again.
- Episodes close with notes (synthesized when the model skips
  finish_episode) and continuity held: each episode picked up the live
  phone/email thread from the previous note without re-deriving it.
- CEO compactions since: **0** (there is nothing left to compact).
- Defect found live and fixed: the nothing-dispatched instant wake
  re-opened an episode every ~30s when the CEO declined to dispatch — a
  re-orientation spin. The wake is now single-shot per idle stretch.

## Phase 3 — ownership routing
Objectives can be owned by a manager. A worker's report on an owned
objective routes to the owner (as a background review task) instead of
the CEO; the CEO receives the owner's summary and anything marked
ESCALATION. Guards: never route a manager's own report, never to a
missing/busy/non-manager owner, and firing a manager reverts their
objectives to CEO routing — a dead owner can't swallow reports.

## Test suite
31 unit tests pass, covering: observation-tool classification, episode
note round-trips, board ranking/staleness/blockers/unblocking, owner
set/clear/render/revert, path resolution, empty-content serialization.

## Live confirmation on the final build
- First heartbeat episode closed with a real finish_episode note
  ("GMX EMAIL (#2) — FINALLY EXECUTING: dispatched email-opS...") and
  dispatched work instead of doing it by hand.
- A no-dispatch episode was followed by a clean event-driven hold, not a
  reopen — the spin fix verified in production.
- Dispatches carry objective tags; a premise-verification dispatch went
  out against objective #5 per mandate clause 9.

## What to watch
- Heartbeat episode quality at the 900s cadence.
- First live owned-objective cycle once the CEO assigns an owner.
