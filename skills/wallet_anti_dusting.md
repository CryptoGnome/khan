How to audit the treasury wallet for dust-attack/airdrop tokens against the allowlist, and how to handle unknown mints safely. Load before any P&L computation, transfer, or token interaction.
# Anti-dusting wallet audit — chain is ground truth

Run BEFORE any P&L computation, transfer, or token interaction. Dust attacks airdrop frozen/poisoned tokens; wallets get drained by "helpfully" cleaning them up.

## Procedure
1. Read-only audit via the private RPC: list every token account of the treasury under BOTH token programs (Token-2022 AND classic SPL — getTokenAccountsByOwner must be run against both), tag each mint ALLOWED / HOSTILE / UNKNOWN against the allowlist.
2. Zero unknowns → clean, proceed.
3. Unknown found → DO NOT interact: no swap, no approve, no transfer, no close, no "cleanup". Interacting with a poisoned/frozen mint is how it drains you.
4. Identify READ-ONLY (getAccountInfo, the platform's coins API — never a tx). Classic dust signature: tiny balance, state=frozen, vanity mint suffix, not a real coin on the platform (API 404). Record it in a hostile-tokens table and write a tagged memory so every agent knows.
5. Never count hostile tokens in P&L, holdings, or treasury value.

## Allowlist
Adding a mint is an explicit decision (founder or CEO, after on-chain verification) — never because a token "appeared". Every swap requires the input mint to be on the list.

## Gotchas
- Private RPC by env reference; public RPCs rate-limit the account scans.
- A zero-balance token account may linger (rent-reclaimable) — ignore for value, but don't close unknown mints (close = interaction).

## OUR INSTANCE
Record here: the allowlist table location, currently allowed mints, and known hostile mints with dates.
