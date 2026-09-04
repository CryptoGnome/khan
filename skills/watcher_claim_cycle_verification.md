Load after ANY income/claim event: the completion flow after an automated claim — verify tx ground truth, verify-or-execute the buyback, backfill the stake column, pay the founder, regenerate DB-derived copy, disclose. Includes restart-mid-cycle detection, because skipping a step that never ran is as wrong as doing it twice.

# claim-cycle verification — completing an automated income cycle

An automated claim routine does most of the work and then, occasionally,
dies halfway through it. The whole point of this flow is the judgment in the
middle: deciding whether a money step already happened, from evidence, before
either repeating it or skipping it.

## When to use
After any claim event lands — a routine alert, a `CLAIMED` line in the claim
log, a new income row in the books, or a health-check alert.

## The trap this skill exists for
A restart can kill the routine between writing the income row and executing
the buyback and payout. The books then show income and nothing else: no
buyback pair, no payout row, a stale position snapshot, and a state file that
says the deferred buyback is zero. **In that state the buyback was never
executed and you must execute it.** Trusting "the routine auto-executes" and
skipping it silently loses the company that buyback.

Both errors are real-money errors. Double-buying is not the only failure
mode; a never-done buyback is the other one, and it is quieter.

### Verify-before-skip checklist — evidence, not assumption
- Do the books hold BOTH buyback rows for this claim? Missing -> not done ->
  execute it.
- Did the receiving token account increase by roughly the buyback amount
  against its pre-claim balance? A matching delta is the proof.
- Is there a founder-payout row for this claim? Missing -> not done ->
  execute it.
- Does the state file say the claim drained with a zero deferred amount?
  Then the claim landed and completion is pending, not done.

If the evidence says the routine completed, skip to verify, backfill, and
whichever of payout or regen is still outstanding. Never re-claim.

## Step 1 — verify ground truth from transaction meta
Never trust an API's self-report. Fetch the claim transaction from the
private endpoint with a confirmed commitment and read:
- the error field is empty, and record the slot;
- net = (post balance - pre balance) at the treasury's account index, minus
  the fee. That number, not the API's, is the claim amount;
- the buyback swap transaction exists as its own signature and the bought
  token actually arrived.

## Step 2 — verify or execute the buyback
If the rows and the balance delta are missing, execute the buyback now at the
policy share of the claim net: quote, swap, then write both ledger rows (one
negative in the spent asset, one positive in the bought asset) with a
disclosure before and after.

### Income-row discipline
Normal path: the routine wrote the income row — verify it against ground
truth, do not insert it again. Repair path (the restart killed the routine
before even that row): insert it once, with a note carrying the transaction
id, the slot, and the error field, so the row is verifiable later. One row
per claim, ever.

## Step 3 — backfill the stake column
Re-run the stake backfill so the maximum stake figure on the buyback rows
equals the on-chain balance of the holding account. Verify that the position
snapshot, the books maximum, and the chain all agree.

## Step 4 — pay the founder
Payout is the policy share of the claim net. Load the payout skill FIRST: a
dust test, confirmation that the dust landed, a disclosure before, the send
to an address verified character by character, a disclosure after, the ledger
row, and the position update.

## Step 5 — regenerate the public copy
Re-run the DB-derived generators; they pick up the new figures on their own.
Then run the copy-figures audit and require exit 0. The freshness check will
alert until the regeneration completes — that alert is the trigger, not a
fault. If the public page carries claim or treasury figures, refresh its
static head too; only the live spans update themselves.

## Step 6 — close the loop
Compute and remember the post-claim lifetime aggregates. Disclose the payout
and the totals publicly and in the run log (the routine already disclosed the
buyback). Confirm the system is green, check the vault, and predict the next
cycle.

## Pitfalls
- A balance read taken seconds after a finalized send can still show the old
  number. Authority is the transaction's pre/post balances or its signature
  status, never a single live read.
- Backfill the stake column on every buyback pair or the maximum goes stale.
- Restart mid-cycle is not a rare event. Run the checklist every time.

## Verification
Done means: ground truth matches the booked income row; buyback rows and the
on-chain delta agree; the payout row exists with a confirmed signature; the
copy audit exits 0; and the aggregates are disclosed.

## OUR INSTANCE
Record here: the policy shares (buyback / retained / founder), the routine
name and cadence, the holding account and treasury addresses, the generator
script names, and the current lifetime aggregates with their date.
