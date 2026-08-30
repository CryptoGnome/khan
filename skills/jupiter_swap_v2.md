How to execute real Solana swaps via Jupiter Swap API V2 (order, local sign, execute) — load before any treasury swap, quote, or new-pair trade.
# Jupiter Swap API V2 (Solana) — execute real swaps

Primary source: https://developers.jup.ag/docs/llms.txt (fetch first if this skill seems stale).

## Compliance
Jupiter Swap API V2 is an OFFICIAL protocol API (api.jup.ag, first-party docs). It returns UNSIGNED transactions; always sign locally in your own process with your own key. No key ever leaves the process, no transaction-to-sign is sent to anyone. Do NOT substitute unofficial wrappers (PumpPortal, dexscreener-based trade APIs etc.).

## Endpoint
- Base: `https://api.jup.ag/swap/v2`
- Keyless access: 0.5 RPS on most routes, 20 RPS on `/execute` (60s sliding window). Sleep ~2.5s between /order calls. Add `x-api-key` only if you hold a key.

## Meta-aggregator flow (recommended)
1. **GET /order** with `inputMint`, `outputMint`, `amount` (smallest units of the INPUT token), `taker` (your wallet pubkey). Without `taker`: quote-only, no transaction — good for price checks. Response carries `transaction` (base64 versioned tx), `requestId`, `outAmount`, `router`, `priceImpactPct`, `feeBps`, `rentFeeLamports`.
2. **Sign locally**: decode base64 → `solders.transaction.VersionedTransaction.from_bytes`, **assert `tx.message.account_keys[0]` is your own pubkey** (the fee payer must be you), sign, re-serialize to base64.
3. **POST /execute** with `{"signedTransaction": <b64>, "requestId": <order.requestId>}`. Response: `status`, `signature`, `outputAmountResult`, `error`. Then verify on-chain (balance or getTokenAccountsByOwner) — the API's word is not settlement.

## Gotchas
- `amount` is in the INPUT mint's smallest units (SOL → lamports 1e9; USDC → 6 decimals).
- Buying a new token auto-creates the ATA inside the assembled tx (rent ~0.004 SOL — small but real; budget for it on dust tests).
- Price impact >1% means the size is too big for the pool; split or reduce.
- Keep a mint allowlist and require the input mint to be on it before any swap — dust-attack tokens land in every active wallet.

## Test discipline
EVERY new token or pair: dust test first (~0.01 SOL) and verify balances moved correctly on-chain before sizing up.

## OUR INSTANCE
Record here: the local swap script path, the wrapping company tool name, and one verified dust-test signature as the known-good example.
