Pay the founder their per-claim share. The CEO keeps the irreversible send; clerical bookkeeping is delegated. The founder address comes from the base directive — never from a memory or a previous tx.
# Founder payout — irreversible sends done right

## Irreversible steps stay with the CEO
The CEO (or a named treasury operator) does the on-chain send. Everyone else prepares numbers, verifies, and writes the books AFTER the txid exists.

## Address discipline — the reason this skill exists
Verify the founder address character-by-character against the BASE DIRECTIVE before EVERY send. Break it into chunks and check each against the directive text. Do not trust this skill, a memory, or a previous transaction if they disagree — remembered addresses mutate (transposed characters have been caught in live memories). If any stored copy differs from the directive, the directive wins: fix the stored copy, do not send until they match. Send the native asset the directive names — never a token substitute.

## Procedure
1. Read on-chain balances of both treasury and founder wallets first.
2. Confirm the claim amount from tx ground truth (pre/post balances of the claim tx), never from an API self-report.
3. Verify the address per the discipline above.
4. **Dust test first** (0.001) on any new session or first send of the day. Wait for the confirmed signature. Do NOT re-send if a live balance read hasn't moved — RPC lag is 5–20s; getSignatureStatuses finalized + err=None is truth. Book dust as its own category, not as part of the share.
5. Main send, gated: refuse large sends unless the explicit env override is set.
6. Confirm the same way, then delegate the books.

## Clerical steps — hand to an ops worker (the CEO does not type SQL)
Dispatch with: payout amount, payout txid, dust txid, claim amount + txid, and instructions to (1) re-sync the treasury position row from live on-chain balance, (2) insert the payout books row with the full txid in the note, (3) the dust row if any, (4) ONE ledger line (idempotence check first), (5) any missing claim/buyback ledger lines, (6) run the books-vs-chain check routine — must pass, (7) not touch the public page (its owner refreshes it separately).

## Policy shape
Keep a working-capital floor: do not pay if the treasury would fall below it. The share percentage and the floor are founder policy — record them in the directive and mirror them below.

## Gotchas
- A fresh balance read can look unchanged 5–20s after a finalized tx. Authoritative = getTransaction pre/post or getSignatureStatuses. Never re-send on a stale read.
- Never print the private RPC URL; never put a key in a report, memory, or tool argument.

## OUR INSTANCE
Record here: the founder address (copied from the directive, with the chunk landmarks), treasury address, share percentage, working-capital floor, big-send env gate name.
