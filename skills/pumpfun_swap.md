Buy/sell pump.fun tokens (bonding curve or post-graduation AMM) via the official /agents/swap API. Load before any pump.fun token buy, sell, or position exit.
# pump.fun swap — official API path

## The only path
`POST https://fun-block.pump.fun/agents/swap`. Same first-party trust model as create-coin: the backend builds the FULL tx (bonding-curve vs AMM decided automatically from coin state), you verify + co-sign locally, your key never leaves the machine. Do not branch on programs manually.

Request (buy): `{"inputMint": "<native SOL mint>", "outputMint": "<TOKEN_MINT>", "amount": "<lamports>", "user": "<YOUR_PUBKEY>", "slippagePct": 2, "feePayer": "<YOUR_PUBKEY>", "frontRunningProtection": false, "tipAmount": 0, "encoding": "base64"}`. Sell: swap the mints, `amount` in token smallest units (6 decimals for pump tokens). Response: `{transaction, pumpMintInfo: {hasGraduated, expectedOutAmount}}`.

## Critical gotchas (all proven on-chain)
- ALWAYS pass `"encoding":"base64"` in the request AND in every RPC send/simulate call — mismatched encodings fail.
- NEVER trust `token_program` from the HTTP API — fetch the mint account on-chain and use `.owner` (pump tokens can be Token-2022). Derive the ATA with the mint's ACTUAL token program; the wrong one yields "ATA not found".
- Signatures are over the raw wire message INCLUDING the 0x80 version byte — raw byte surgery, never rebuild MessageV0.
- SOL = 9 decimals, pump tokens = 6.
- Public RPC often can't send or lags — use your private RPC (env by reference). BlockhashNotFound on sim = send anyway with skipPreflight.
- Coin state: `!complete` → bonding curve; pool set → AMM; neither → migrating, wait + retry.

## Position discipline
- Only swap mints on your allowlist (see `wallet_anti_dusting`).
- No silent selling of a flagship position: disclose publicly BEFORE reducing; never dump into your own community.
- Sanity-check price via a Jupiter quote or the response's `expectedOutAmount`; simulate on the private RPC before sending.
- Book both sides, sync the positions rows, one ledger line per txid (idempotence check first).

## OUR INSTANCE
Record here: flagship mint + current stake, the pre-built sell wrapper tool name, allowlist location.
