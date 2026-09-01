Execute a checkpoint kill on a staged experiment position: clock read, pre-sell ledger intent, sell, parse the transaction for ground truth, atomic booking, treasury position sync, settle the monitor. Load BEFORE any kill-clock fire, kill exit, or kill booking — the sequence begins at the clock read, not at the sell.

# launch_kill_sequence — exits are booked, not just executed

A staged experiment position is opened with kill criteria already written
down, so the hard part of the exit is never the decision — it is finishing the
bookkeeping before something interrupts. A sell that is confirmed on-chain but
unbooked is ledger drift somebody has to reconcile later against a wallet that
has already moved on. A sell that is booked but unannounced is a perfectly safe
place to be interrupted. That asymmetry sets the whole order of operations
below: book immediately, report afterwards.

## When to use
- Any staged experiment position hitting a scheduled checkpoint with kill
  criteria armed (volume, holders, price, graduation gates), or a staged
  take-profit exit.
- Load BEFORE the clock fires.
- NOT for discretionary trades, and NEVER for the flagship position — that one
  is not traded.

## Procedure

**Phase 1 — fire and book.**
1. At the checkpoint instant, run the clock read: current venue state against
   the written criteria. Criteria met, kill. The upside gate tripped instead
   (the position graduated), take the staged win contingency. Criteria missed,
   no trade — note it and wait for the next checkpoint.
2. Write the ledger INTENT line BEFORE the sell, carrying the criteria and the
   clock read that justified it. This is not ceremony: it is what makes the
   ordering — decision before action — provable afterwards from row order
   alone.
3. Sell the whole bag in one call through the venue's first-party path.
   Confirm the transaction on-chain.
4. **Book now**, before reporting, notifying, or waiting on anything:
   a. Parse the confirmed transaction for the real treasury delta and fee;
      gross is delta plus fee. NEVER book from the swap tool's expected-output
      estimate — the estimate is a forecast, the parse is the evidence.
   b. Write the pnl row, close the position, and record the closed position in
      one atomic booking step keyed to the transaction id.
   c. Check that the intent row still sorts before the sell row.
   d. **Sync the treasury position row to a fresh on-chain balance read**, then
      run the chain-reconciliation routine and confirm it exits clean. This is
      part of booking, not optional cleanup.
5. Only then settle the monitor (one checkpoint run so it marks alerted and
   stops firing), append the executed block to the verdict file, and hand the
   milestone to whoever owns the public voice.

**Phase 2 — report.** The transaction id, parsed gross and net, PnL against
entry cost, booking confirmation, and the reconciliation routine's exit status.

## Pitfalls
- **An iteration or shell cap will cut the sequence in half.** It has happened
  twice in one day at this company: one kill left intent-written but unbooked
  for hours, another left the sell confirmed but unbooked. Size the work into
  chunks that fit the cap and book at the phase boundary — that single ordering
  rule is what makes an interruption harmless.
- **A claimed sync is not a sync.** One kill was booked correctly in every
  ledger table but left the treasury position row at its pre-kill value; the
  reconciliation routine alerted on the drift while the worker's report claimed
  the sync was done. It was checkable in one SELECT. Never assert a write you
  have not read back.
- An unexplained treasury delta at kill time is usually another lane's
  booked-but-unseen outflow — a payout, a funding leg, a fee claim. Decode the
  transaction's own amounts and destinations, and check the recent ledger rows,
  before escalating it as an unknown actor.
- Settle the monitor even on a kill; an unsettled monitor re-alerts on a dead
  position forever.
- The upside contingency flips only on a true graduation read at the clock
  instant. A near miss is a kill, per the criteria as written.
- Booking helpers that use relative database paths must be run from the
  workspace root; run elsewhere they fail with a missing-table error that looks
  like a schema problem and is not.

## Verification
The pnl row carries the PARSED delta, not the estimate; a closed-position row
exists; the intent row sorts before the sell row; the treasury position row
equals a live balance read; the chain-reconciliation routine exits clean; the
monitor is marked alerted; the verdict file is marked executed. Any one of
these missing means the sequence is unfinished, whatever the report says.

## OUR INSTANCE
Record here: the paths of the clock-read and booking helper scripts and their
argument order, the verdict file naming convention, the shell timeout and the
chunk size that fits inside it, the treasury position row id that must be
synced, and the precedent kills with their outcomes (so the lane's cost is
judged against actuals rather than hope).
