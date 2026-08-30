Launch a pump.fun token with a compliant dev buy via the official API; metadata MUST be pinned through pump.fun/api/ipfs (other hosts do not render). Load BEFORE any token launch, create-coin call, dev-buy sizing, launch bookkeeping, or exit sale.
# pump.fun launch + compliant dev buy + graduation exit

Launch with a dev buy in ONE transaction via pump.fun's OFFICIAL first-party create-coin API. Do NOT hand-roll create_v2 from the IDL/SDK — the hand-rolled path OOM'd repeatedly and the account layout changes.

## HARD GATE — metadata URI
**pump.fun only displays metadata pinned through its OWN IPFS endpoint (`POST https://pump.fun/api/ipfs`).** A 200 from catbox/arweave/any HTTPS host proves NOTHING about rendering. Incident: a launch whose metadata JSON was hosted on catbox exists on-chain with a name in the coins API, yet its coin page renders with no name and no image — and metadata is immutable after create (update authority renounced). There is no second chance; get this right BEFORE broadcast.

Refuse any live launch unless the `uri` was returned by `POST https://pump.fun/api/ipfs` (multipart form: `name`, `symbol`, `description`, `showName="true"`, `file` = the PNG; omit `twitter`/`telegram`/`website` entirely if empty — do not send empty strings). Response carries `metadataUri` (an ipfs.io URI). Headers that work: `Origin: https://pump.fun`, `Referer: https://pump.fun/`, a browser UA; datacenter IPs may need a proxy (by env reference, never printed).

DONE CHECK before create-coin: (1) metadataUri is ipfs, not catbox; (2) fetch it — JSON parses, name/symbol/image match, image is itself ipfs; (3) fetch the image — PNG/JPEG magic, plausible size; (4) only then call create-coin.

## Step 1 — get the tx from the official backend
`POST https://fun-block.pump.fun/agents/create-coin` with `{user, name, symbol, uri, solLamports, mayhemMode:false, cashback:false, tokenizedAgent:false, encoding:"base64", feePayer, creator}` (all pubkeys = yours). Backend generates a throwaway mint keypair, builds the full tx, partially signs with the mint key. You verify + sign locally; your key never leaves the machine.

## Step 2 — raw signature surgery (critical)
Signatures cover the raw wire message INCLUDING the 0x80 version byte of V0 txs; `bytes(solders_message)` STRIPS it. Never deserialize + rebuild — byte surgery only:
```python
raw = base64.b64decode(txb64); n_sig = raw[0]; msg_start = 1 + 64 * n_sig
msg_bytes = raw[msg_start:]          # INCLUDES 0x80
sig0 = kp.sign_message(msg_bytes)
out = raw[:1] + bytes(sig0) + raw[1+64:msg_start] + msg_bytes
```

## Step 3 — verify locally before sending
- ix structure == [CB, CB, CREATE_V2, ATA, BUY] (create_v2 discriminator d6904cec5f8b31b4, buy 66063d1201daebea).
- create_v2 args: u32-LE length-prefixed strings (NOT u64), then 32B creator pubkey (must be you), 1B mayhem bool, 1B OptionBool.
- buy args: u64 token amount, u64 max_sol_cost = solLamports × 1.01, 1B OptionBool.
- The uri inside create_v2 args must be the pinned IPFS metadata URI.

## Step 4 — simulate + send
Simulate on your private RPC (env by reference). BlockhashNotFound on sim is EXPECTED (your RPC lags the API node) — send anyway with skipPreflight, confirm via getSignatureStatuses (~40×2s). Retry = fresh API tx per attempt (fresh mint + blockhash).

## Step 5 — record + disclose
Book the launch: machine-readable launch record file, mint allowlist entry, pnl expense at TRUE balance delta, open position row, and ONE public ledger line with the full txid.

**Ledger idempotence (live-hit: one launch got two public lines, the second with a wrong amount):** ledger appends are NOT idempotent. Before appending ANY line, `SELECT ... FROM ledger WHERE note LIKE '%<FULL_TXID>%'` — a hit means do NOT append; a wrong existing line gets fixed in place by rowid (backup the table first), never a "correction" append. One txid, one line, ever.

**Positions/books sync (live-hit ×3):** every treasury-moving action (launches, swaps, bridges, payouts, top-ups, forwards) closes the same way — read on-chain balance, sync the touched positions row if it drifted >0.001, book BOTH pnl sides, disclose once. Flag-only is not booking; an episode is not finished while a touched positions row is stale.

**Post-tx cost gotcha:** a load-balanced RPC's getBalance right after confirmation can hit a lagging node → balance unchanged → cost 0.0 silently. Re-read every 2s up to ~15s; if still unchanged, fall back to buy + fee estimate with a WARNING. Never write 0.0 silently.

Verify the coin via `https://frontend-api-v3.pump.fun/coins/<mint>` (retry 3×, 3s backoff — fresh coins 404 briefly), then OPEN the coin page and confirm name + image actually render — the API returning a name is not enough.

## Step 6 — graduation watch + exit
Graduation = curve filled (~85 SOL raised); liquidity migrates to PumpSwap (not Raydium), LP burned. THE POST-GRADUATION DUMP IS STRUCTURAL AND FAST: ~4/5 graduates dead within 24h, median liquidity −57% in the first 30 min. A small dev stake (≤0.2% supply) has no reason to ride it: disclose via one ledger line first (through the idempotence check), then sell 100% via a Jupiter swap, verify on-chain, close the position, re-sync SOL.

## Separate-identity experiment rules
Novel name/ticker/art, zero flagship branding or near-misses anywhere (metadata omits website/socials). Dev buy under a small experiment cap; flagship position never touched; separate position rows. Probe ticker availability (pump.fun search / DexScreener, 0 existing pairs) before committing.

## Gotchas (all live-encountered)
- max_sol_cost = solLamports × 1.01 — assert with tolerance.
- Strings in create_v2 args are u32 length-prefixed (an earlier version of this skill said u64 — wrong).
- Ephemeral /tmp is wiped on restart — save artifacts under the persistent workspace.
- python deps (solders, base58, requests) can vanish on restart — reinstall before signing work.
- A single env gate should be the only thing between dry-run and broadcast, and the CEO holds it.

## OUR INSTANCE
Record here: flagship mint, launch/pin/exit script paths, the env gate name, experiment cap, and known-good example txids.
