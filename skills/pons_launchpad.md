Launches and trades tokens on pons, the launchpad of Robinhood Chain (the pump.fun of Robinhood's L2): v1 is OPEN today (0.0005 ETH fee, locked Uniswap v3 pool, 70% of pool fees to creator); v2 is whitelist-gated bonding curves. Load BEFORE any pons launch, trade, or creator-fee claim on Robinhood Chain.

# pons launches on Robinhood Chain

pons runs on Robinhood Chain — Robinhood's Arbitrum-stack L2. Two versions,
one decision: v1 launches are open to anyone NOW; v2 (bonding curves that
graduate to Uniswap v4) is whitelist-only. Use v1 until `canLaunch` on the
v2 factory returns true. This is a LOW-FREEDOM procedure: real money on
young protocols. Docs (fetch live before first use — contracts and rules
move): https://docs.ponsfamily.com (v1), /v2, /llms.txt.

## Standing warnings — read every time
- Cap exposure on either version: launch fee + minimal dev-buy only; never
  park treasury in a pons pool or curve.
- v2 is UNAUDITED (three audits in progress, none published) and its
  launching is currently closed to the public: `canLaunch(ourAddress)` on
  the v2 factory must return true FIRST — if false, the v2 path is dead
  until whitelisted (contact@ponsfamily.com); do not probe around it, use v1.
- v2 SNIPE TAX: buys in the first 5 seconds are taxed from 99% decaying to
  zero. Never buy at a v2 launch except via the LaunchAndBuy router (its
  recipient is auto-exempt).

## Chain facts
- Robinhood Chain: chain id 4663, gas token ETH,
  RPC https://rpc.mainnet.chain.robinhood.com,
  explorer https://robinhoodchain.blockscout.com.
- Getting ETH there: LI.FI routes to Robinhood Chain (also: canonical
  Arbitrum bridge, Relay, Across, Stargate). Follow bridge_hygiene —
  dust-test first, verify on the destination RPC.

## v1 — the open path (use this today)
Contracts (verify on the explorer before signing; ABIs from Blockscout):
- Factory `0xA5aAb3F0c6EeadF30Ef1D3Eb997108E976351feB`
- Locker `0x736D76699C26D0d966744cAe304C000d471f7F35`
- Swap Router `0xCaf681a66D020601342297493863E78C959E5cb2`,
  Quoter V2 `0x33e885eD0Ec9bF04EcfB19341582aADCb4c8A9E7`,
  WETH `0x0Bd7D308f8E1639FAb988df18A8011f41EAcAD73`

Mechanics: launch fee 0.0005 ETH; fixed 1e9 supply mints straight into its
own locked Uniswap v3 WETH pool (1% fee tier) in the launch transaction —
no curve, trading is live immediately via the swap router. Graduation at
4.2 ETH paired (`factory.graduationStatus(token)`) changes nothing
structurally — same pool, it's a milestone flag. Creator gets 70% of pool
fees (protocol 30%); fees accrue in the locked position, payout wallet via
`locker.feeRedirects(token)`, claimable from the pons interface, and
unclaimed fees are auto-routed to the payout wallet.

v1 launch procedure:
1. Wallet ours + vaulted, funded with ETH (gas + 0.0005 fee + dev-buy).
2. Pull the factory ABI from the explorer to get the exact create signature
   (metadata rides in the token: name/symbol/logo/description/socials).
3. Launch protection window: launch block allows only the creator's initial
   buy; next block caps 5% supply per wallet — make the dev-buy IN the
   launch transaction, size modestly.
4. Verify: token + pool live on the explorer (token's `liquidityPool()`),
   metadata renders on https://www.ponsfamily.com/launchpad.
5. Trade quotes via Quoter V2, execution via the swap router; price from
   the pool's slot0 (both sides 18 decimals, no scaling).

## v2 — whitelist-gated bonding curves (when canLaunch is true)
Contracts (verify against live docs before signing):
- Factory `0x7eD598BcEf8bd9Edd8C97A195C6d13f40801EC7e`
- LaunchAndBuy router `0xe33E9E479dF8802cb0866d5d05258bEc4cF62948`
- FeeEscrow `0xd3AFEB2a57f70eF218Aa82451c51B2fb0416Ac9e`
- Meme hook (v4 pool fees) `0xE5e702641Ea86F4ae6cC3cDaeD2B886f976Be044`

## Procedure — v2 launch
1. Gate: `factory.canLaunch(us)` must be true; wallet is ours, key vaulted,
   funded with ETH for gas + launch fee (`factory.launchFee()` — query it,
   never hardcode).
2. Pick config + pair: enumerate `launchConfigCount()` / `getLaunchConfig(id)`
   (want `enabled`), pair with native ETH (zero address) unless a reason not
   to; `approvedPairTokens(addr)` gates ERC-20 pairs.
3. Pin economics: `previewLaunchEconomics(configId, pairToken)` → pass the
   result as `expectedEconomics` so a config change reverts the tx instead
   of silently changing terms.
4. Launch: `launchToken(TokenParams, configId, pairToken)` with launchFee as
   value. TokenParams: name/symbol/logo/description, socials, our
   creatorFeeRecipient, creatorTaxBps (≤ `maxCreatorTaxBps()`; modest —
   greed kills volume), buybackEnabled, expectedEconomics, unique salt.
   For a launch + dev-buy in one atomic tx use the router's `launchAndBuy`
   (recipient snipe-exempt).
5. Verify on the explorer: token + curve addresses returned, curve holds
   supply, metadata renders on https://www.ponsfamily.com/launchpad.

## v2 curve math (quote before any trade)
- `curve.getReserves()` → constant-product, integer order:
  out = in*reserveOut/(reserveIn+in). Fees (`feeBps` + `creatorTaxBps`) hit
  the quote leg; add `currentSnipeTaxBps(recipient)` to buy quotes.
- `sell` reverts once `readyToGraduate()` — graduation auto-fires inside the
  buy that empties `sellableTokens()`; if `AutoGraduationFailed` fires,
  `createGraduatedPool(token)` is permissionless and retryable.
  `factory.getLaunchedToken(token).phase`: 0 curve, 2 pool live.

## v2 fees — where the revenue is
- Curve-phase fees credit FeeEscrow: `balanceOf(us)` / `balanceOfToken(us,
  asset)`, withdraw with `claim()` / `claimToken(asset)`.
- Post-graduation the v4 hook accrues `pendingFees(poolId, currency)`;
  `sweepPoolFees(poolId, minConversionQuoteOut, minBuybackTokensOut)`
  converts and credits escrow — set real minimums, it swaps.
- Book every claim in the ledger like pump.fun creator fees.

## Read-only market data (no wallet needed)
https://www.ponsfamily.com/api/pons-launches, /api/pons-token/{token},
/api/pons-market/{token} — use these for scouting before spending gas.

## Verification
Done means: token + pool/curve live on the explorer, metadata renders on
the launchpad site, dev-buy (if any) landed inside the protection rules,
fees verifiably accruing to OUR recipient, and the launch booked (fee paid,
addresses, version used). A v2 launch that cannot pass `canLaunch` is
reported as blocked and rerouted to v1, not retried.
