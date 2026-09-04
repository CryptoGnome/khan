Load before touching the claim cycle: the claim is a routine and not a daemon, completion is a separate shell routine, ordinals are derived from the ledger and never from a note, and the buyback is linked by row order and never by transaction id. Load also after any threshold or cadence change.

# fee_claim_scheduler — running an automated income cycle

The claim cycle collects income on a timer, buys back on policy, and hands
the irreversible leftovers to the CEO. Everything below was earned by a
defect that reached the books.

## When to use
Before editing the claim routine, its completion handler, or any threshold it
reads; before diagnosing a missed or duplicated claim; and after any change
to a threshold, cadence or policy figure.

## Shape of the system
1. **The claim routine** (on a fixed cadence, lock-guarded so two ticks never
   overlap) reads every fee vault, and if the total clears the threshold it
   claims through the official API, co-signing locally and sending through
   the private endpoint. Below threshold it writes the state file and stays
   silent — no disclosures for a skip.
2. **The completion handler** runs on the same cadence as a SHELL monitor —
   command populated, no agent and no task — so the scheduler executes it
   rather than dispatching it to a model. It picks up after each claim and
   automates verify, buyback, backfill, payout staging, regeneration and
   audit, writing one action file containing ONLY the irreversible leftovers.
3. **An emergency trigger** (read-only, never claims) covers the volume event
   where vaults cross the threshold between ticks with a failed or absent
   claim. Only a completed claim suppresses it; a fresh skip must not.

There is no daemon. A running claim daemon is a health violation — the
routine imports the cycle function directly so there is one source of truth.

## The threshold is CODE-truth
The operative threshold is the constant in the claim module, not the figure
in a skill, a state file or a plan.

**Text-truth rule:** when a threshold or cadence figure lives in more places
than the code, any of them can go stale. After ANY change to one, grep the
figure across the routines directory, the module sources and the staged JSON,
and update every carrier in the same episode. A stale figure in one carrier
re-propagates the false premise on the next read.

## The ordinal law
Claim notes do not necessarily carry a claim number. Any label or staging
note that interpolates one resolves it through a single function:
1. A note-borne number wins when present (legacy rows carry them).
2. Otherwise COUNT the income rows up to and INCLUDING this row's id. The
   `id <= row_id` bound is required — without it a later claim inflates the
   ordinal of an older un-numbered row.
3. If the database cannot be read, emit the string `unnumbered`. Never the
   literal `None` — that reached a live ledger row.

Watchdog: if a claim's ordinal disagrees with the public page's claims count,
**the page's canon governs**. Investigate the predicate; do not renumber the
books to match a script.

## Handler stages
- **VERIFY** — on-chain treasury delta against the booked income row, within
  a tolerance. A mismatch alerts and stops; it never proceeds.
- **VERIFY-OR-EXECUTE the buyback** — **link by ledger row ORDER, never by
  transaction id.** Buyback notes carry the SWAP signature, not the claim's,
  so a claim-id probe finds nothing and executes a second time. That cost a
  real duplicate swap. The correct predicate is: rows between this income row
  and the next one. All present -> skip; missing -> execute through the
  proven path with disclosures either side. Grab the claim lock
  non-blockingly and skip the tick if the claim cycle holds it.
- **BACKFILL** — refresh the stake column from the live account, with one
  retry for a stale read, and tolerate sub-unit rounding dust rather than
  failing on it.
- **PAYOUT STAGING** — write the staging row and the exact send line into the
  action file. Probe for idempotence on BOTH the swap-id fragment and the
  income row reference, since payout notes historically reference the row.
  The send itself stays CEO-only.
- **REGEN** — run the DB-derived generators against staged files only. The
  handler never touches the live page.
- **AUDIT** — run the copy audit and flag a nonzero exit in the action file.

Live runs append one bookkeeping row per claim as the idempotence marker. A
dry-run environment variable must rehearse every stage and write nothing.

## What still needs eyes after every claim
Read the latest action file, do the founder send (payout skill: dust test,
address verified character by character), and refresh the page's static head.
If the handler alerted, run the verify-before-skip checklist by hand — the
handler is automation, not authority; ground truth still governs.

## Zombie and process gotchas
Detached processes re-parent to PID 1 when their spawning shell exits.
Killing one leaves a Z-state entry that KEEPS its command line in `/proc` —
count only S-state processes. And before diagnosing a "crash loop", check the
spawn history in the run log: a chain of PIDs that looks like a supervisor
restarting something has, at least once, been nothing but a sequence of
deliberate kill-and-restart commands. PID 1 here is not a supervisor and does
not even reap zombies; old PIDs in `/proc` are corpses, not processes.

## Test discipline
Never test the buyback or claim live. Copy both databases to scratch,
monkeypatch the public-say function to a list, stub the send subprocess to
return a plausible success payload, and stub the status call to finalized.
Assert: both disclosures written, both ledger rows written with the
signature, position delta correct. Then run the real path — dry first.

Never call a writer script's `main()` in-process without an explicit argv: an
empty argv can default to the live path. That was paid for once.

## Verification
One claim event produces exactly: one income row, one buyback pair, one
payout row, one bookkeeping marker, and an action file. Any missing member of
that set is an incomplete cycle, not a quiet success.

## OUR INSTANCE
Record here: the threshold constant and its file and line, the routine and
handler names with cadences, the policy shares, the lock path, the state and
log file paths, and the forbidden legacy daemon commands.
