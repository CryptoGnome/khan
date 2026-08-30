Claim pump.fun creator fees for coins the company created via the official collect-fees API, then apply the capital-allocation split (buyback / retained / founder). Load before ANY creator-fee claim or claim-cycle work.
# pump.fun creator fees — claim + allocation split

## Endpoint
`POST https://fun-block.pump.fun/agents/collect-fees` body `{mint, user, frontRunningProtection:false, tipAmount:0, encoding:"base64"}` → `{transaction, creator, isGraduated, usesSharingConfig}`. Verify + co-sign locally, send via your private RPC (env by reference).

## The cycle (run as a zero-cost ROUTINE, not a daemon)
1. Read both vaults: pump `creator-vault` native SOL (PDA: creator-vault + creator under the pump program) and, for graduated coins, the AMM creator_vault WSOL ATA.
2. Total claimable ≥ threshold (default 0.5 SOL, to beat tx costs) → claim; below → silent skip to a state file, no disclosures.
3. On a confirmed claim, execute the allocation split immediately (see below).
4. Health: a companion routine verifies the last claimed tx has its books row, the ledger is intact, and the state file is fresh.

## Ground truth (DB-first)
NEVER trust the API's self-reported amount or a fresh balance read on a lagging node. Truth = preBalances/postBalances from getTransaction (maxSupportedTransactionVersion:0). NEVER write a 0.0 books row or a ledger line from a failed lookup.

**Safety gotcha:** if a claim returns UNVERIFIED or 0 (meta read failed), do NOT buy back, do NOT write phantom rows, do NOT log a ledger claim line. Leave the txid in the log, mark pending, and complete the cycle once ground truth resolves — the split needs the real amount.

## Allocation split (set the percentages as founder policy)
Example policy: 40% of every claim → open-market buyback of the flagship token; of the remaining 60%, half retained as working capital, half paid to the founder. Whatever the split, each leg follows the same discipline:
- **Claim** → books row + ONE ledger line with the claim txid.
- **Buyback** → swap via Jupiter, verify the token landed on-chain (mind Token-2022 ATAs), two-sided books rows, cumulative stake recorded from on-chain truth, ONE ledger line with txid + amount + % of supply.
- **Payout** → see the `founder_payout` skill (dust test, char-by-char address check, CEO holds the send).
Every ledger append goes through the idempotence check first (`note LIKE '%<txid>%'` — one txid, one line, ever).

## Tx structure (verified on-chain)
ComputeBudget ×2, collect-from-creator-vault, create WSOL ATA, AMM collect, close WSOL ATA. May use pump's shared address lookup table — resolve via getAddressLookupTable. Sign the raw wire bytes including the 0x80 V0 prefix; sim with replaceRecentBlockhash:true; send skipPreflight:true; confirm via getSignatureStatuses.

## Gotchas
- Fees accrue fast during active trading — re-read the vault after each drain.
- Public page aggregates drift after claims — queue a copy refresh with FINAL books numbers.
- Never print the private RPC URL; reference it by env var.
- Killed daemons leave Z-state zombies that keep their /proc cmdline — see `routine_script_pattern`.

## OUR INSTANCE
Record here: the live coin mint(s), the exact split percentages and their directive date, the claim-cycle routine name and interval, threshold, and state-file path.
