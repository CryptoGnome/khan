Buys real-world infrastructure the company cannot get by signup — phone numbers, SMS verification, email inboxes, domains, VPS, prepaid Visa cards — from palmyr.ai, paid per-action in USDC with no human, no card, no API key. NOT social accounts (banned — see Pitfalls). Load when a task is blocked on a phone number or SMS code, needs a domain, a server, or a card number, or when any signup rejects VoIP numbers.

# palmyr.ai — buy infrastructure with USDC, no human in the loop

Palmyr is an agent-native CLI: every action is paid at request time in USDC
over x402 (Solana or Base), the paying wallet becomes the owner, and the
settled tx hash comes back in the response. Discovery commands (`search`,
`info`, `pricing`, `health`) and all local wallet operations are free.

## When to use
- A signup demands SMS verification and rejects VoIP (Bluesky and X both do —
  this is the tested answer to that wall).
- The company needs a domain, an email inbox, a VPS, or a prepaid Visa card,
  and no human is available to buy one.
- Do NOT use for anything a free API or an existing company account covers.
- Do NOT use for social accounts of any platform — see the ban in Pitfalls.

## Quick reference
```
npm i -g @palmyr/cli                      # or: npx @palmyr/cli <cmd>
PALMYR_WALLET_PASSPHRASE="..." palmyr wallet create --name khan-infra
palmyr wallet use <WALLET_ID> --chain solana
palmyr wallet info <WALLET_ID>            # addresses to fund with USDC
palmyr pricing                            # FREE — the live authoritative price list
palmyr phone search --country US          # FREE
palmyr phone buy --country US             # $3.00, returns the number
PALMYR_JSON=1 palmyr phone messages ...   # $0.02, read the verification SMS
```
Published prices (2026-08, verify with `palmyr pricing` before spending):
phone number $3.00, read SMS $0.02, send SMS $0.05, email inbox $2.00,
domain buy $20, VPS deploy $6.00, prepaid Visa = amount + 3% (min $0.50).

## Procedure
1. Fetch the full contract first: https://palmyr.ai/skill.md is Palmyr's own
   agent manifest (install, wallet model, every command). Machine-readable
   docs: https://docs.palmyr.ai/llms.txt. Read before first use — do not
   guess commands.
2. Create a dedicated Palmyr wallet with a passphrase (env var, never argv),
   record the wallet ID and addresses in the company vault, and back up the
   passphrase like the treasury key. Never use `--session-only` (machine-bound,
   unrecoverable).
3. Fund it small: swap treasury SOL to USDC (jupiter_swap_v2 skill) and send
   only what the next purchase needs plus ~$1 margin.
4. Dust-test the pipe: run one cheap paid action first (`phone buy` at $3 or
   an even cheaper read) and verify the result works end-to-end before any
   larger purchase.
5. Buy the real thing. Always run with `PALMYR_JSON=1` and branch on exit
   codes, not stderr text: 0=OK, 2=bad input, 5=network, 6=payment failed
   (wallet balance first suspect), 7=wallet integrity — stop and report on 7.
6. Book every purchase in the ledger: action, cost, settled tx hash (it is in
   the JSON response), and what the company now owns.

## Pitfalls
- Prices in this file go stale; `palmyr pricing` is free and authoritative —
  check it before any spend over $1.
- PURCHASED SOCIAL ACCOUNTS ARE BANNED (X, TikTok, any platform, any vendor).
  Incident: a bought aged X account's original seller can still hold the
  recovery email/phone, and access depends on vendor credentials and a pinned
  proxy the company does not control — it can be reclaimed or locked at any
  moment, making every dollar spent on it unrecoverable. Self-signup with own
  email, own password, own 2FA, and a self-describing handle — or no account
  at all.
- Phone numbers are per-country; some services blacklist entire ranges —
  if a verification fails, `release` the number ($0.01) and buy a different
  country rather than retrying the same one.
- Exit code 6 mid-flow can mean the USDC ran out on the payer chain: check
  `palmyr wallet info` before assuming the service is broken.

## Verification
Done means: the bought asset demonstrably works (SMS code received, post
published, domain resolves, VPS reachable over SSH), the tx hash is booked in
the ledger, and the credentials/wallet are in the vault. A purchase without
all three is unfinished.
