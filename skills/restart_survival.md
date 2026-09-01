Load IMMEDIATELY after any binary restart, or when the task board shows zero background tasks despite dispatches recorded in flight: a restart kills every in-flight dispatch silently, and only disk-written evidence survives. Governs the verify-what-landed triage before rating or re-dispatching anything.

# restart_survival — a restart is a silent mass casualty

A binary restart kills ALL in-flight dispatches mid-run. The task list then
reads zero. There is no error, no report, no marker — the work simply is not
there any more. Evidence a worker already wrote to disk survives; everything
still in a worker's context — the unwritten report, the un-booked row, the
half-finished edit — is gone. Silence after a restart means death, never
completion, and the whole triage below exists because the first instinct
(reading an empty board as "everything finished") is wrong.

## When to use
- Immediately after any restart, or any fresh episode following an
  unexplained gap.
- When the board shows zero tasks but the previous episode recorded
  dispatches in flight.
- Before rating or re-dispatching anything that may predate a restart.
- NOT for a task that merely reports failure — that one ran and lost; this is
  for tasks that vanished without reporting at all.

## Procedure
1. **Treat everything as unverified.** Never read a pre-restart "in flight"
   count as live work.
2. **Verify what landed, before rating or re-dispatching.** Read the cheap,
   silent evidence: routine state files, the DB rows an action would have
   written, the deployed artifact's hash against the approved draft, process
   counts. Each read answers exactly one question — did this specific work
   product exist before the restart?
3. **Rate only what verified.** A pre-restart report describing work may be
   describing a corpse. Spot-check the artifact, not the narrative.
4. **Re-dispatch resume-first, and only the gap.** Name the surviving
   evidence and the verified hole in the task text, so the worker probes
   before it writes and never duplicates. Probe-then-write is not caution
   here: double-booked ledger rows and double-sends are real-money errors.
5. **Live artifacts get triaged first.** If a dead task was mid-edit on a
   deployed page or mid-swap on a daemon, check that artifact's integrity
   before anything else — a half-deploy is public.
6. **Close the loop on the board.** Every affected objective ends the episode
   as verified-done (evidence on disk), re-dispatched resume-first, or
   honestly noted as lost with the reason.

## The doctrine this teaches
Anything worth keeping from a long-running task must be written to disk
INCREMENTALLY — an evidence file after each gate, a DB row as each action
lands. A restart at minute forty erases a write-at-the-end report entirely
and leaves nothing to resume from. Dispatch multi-gate work with "write
evidence incrementally" in the task text; that one sentence is the difference
between a resumable job and a lost one.

## Pitfalls
- No error message ever appears — do not wait for one.
- A worker can report success for work that finished seconds before the
  restart. Still verify: the artifact is the truth, the report is a claim.
- Do not re-run verified work "to be safe". Probe first, execute only the
  verified gap.

## Verification
Triage is done when the board and disk agree: every dispatch from before the
restart is accounted for as verified-done, resumed, or noted lost — and
nothing has been rated that has no surviving artifact.
