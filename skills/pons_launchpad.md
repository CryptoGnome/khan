Launches and trades tokens on pons, the dominant launchpad of Robinhood Chain (~12k launches/day, $100M+ daily volume as of late Aug 2026) — verify contracts ON-CHAIN first (the domain ecosystem is messy and one domain served stale data), then launch via the factory and claim the 70% creator fee share. Load BEFORE any pons launch, trade, or fee claim on Robinhood Chain.

# pons launches on Robinhood Chain

pons is the pump.fun of Robinhood Chain — Robinhood's Arbitrum-stack L2 —
and it is BIG: press (Blockworks, CryptoTimes) credits it with over half the
chain's transactions. This is a LOW-FREEDOM procedure with one rule above
all the others: TRUST ONLY THE CHAIN, because the docs ecosystem around
pons is polluted with clones.

## THE DOMAIN MESS — read first, every time
pons lives across several domains (ponsfamily.com with the docs subdomain,
ponsdotfamily.com, third-party mirrors like ponslaunchpad.com), and on
2026-08-30 one of them (ponsfamily.com) served an api frozen 18 days stale —
which produced a false "dead platform" verdict — while another showed live
launches. Both major domains footer-link the same official X handle
(@ponsdotfamily), so this looks like a messy official-domain situation
rather than a proven clone; but a clone with swapped addresses is the
standard wallet-drain pattern and nothing about the domains rules one out.
Treat every pons website as unverified presentation. Therefore:
- NEVER take a pons contract address from any website, this skill included.
  Derive addresses from the chain: pull recent launch transactions on
  https://robinhoodchain.blockscout.com, find the factory they all call,
  and read its VERIFIED source there. An address that cannot be confirmed
  by live on-chain traffic does not get a transaction signed against it.
- Cross-check activity claims against the explorer, never a site's own api
  alone: a launchpad claiming thousands of launches a day shows them on
  the chain.

## Chain facts (stable, multi-source)
- Robinhood Chain: chain id 4663, gas token ETH,
  RPC https://rpc.mainnet.chain.robinhood.com,
  explorer https://robinhoodchain.blockscout.com.
- Getting ETH there: bridges include the canonical Arbitrum bridge, Relay,
  Across, Stargate; LI.FI indexes the chain but has no Solana leg — a
  SOL-side treasury needs a two-hop (SOL to ETH mainnet, then bridge in).
  Follow bridge_hygiene: dust-test, verify on the destination RPC.

## Protocol shape (v1, as reported — re-verify from the real docs and the
## verified factory source before relying on any number)
- Launch fee ~0.0005 ETH; fixed 1e9 supply minted straight into a locked
  Uniswap v3 WETH pool (1% fee tier); trading live in the launch tx.
- Creator takes ~70% of pool fees (protocol 30%); fees accrue in the locked
  position; graduation milestone at 4.2 ETH paired changes nothing
  structurally.
- Launch-block protection: only the creator's buy in the launch block (so
  the dev-buy rides IN the launch transaction), per-wallet caps the next
  block.
- v2 (bonding curves graduating to Uniswap v4) is whitelist-gated; access
  is a partnership conversation via the project's verified X account —
  route around it with v1, do not probe it.

## Launch economics: the buyer's side comes first (founder 2026-09-01)

Read from the v2 docs directly, so nobody relitigates it from memory:

- **Two separate charges.** The standard trading fee (split pons / creator /
  buyback) and an OPTIONAL `creatorTaxBps` that goes 100% to the creator.
  The docs name the tax as the thing a trader checks before buying: "A
  launch with no creator tax is charging you the standard fee and nothing
  else." So `creatorTaxBps` is ALWAYS 0 on our launches. It is the one
  number that makes our token cost more to trade than the launch next to
  it, and no take rate is worth being the expensive option.
- **The actual split**, from the live FeePolicySnapshot on the meme hook
  (curveFeeBps 100, protocolFeeShareBps 3000, buybackBurnBps 5000) — not
  from anyone's memory: the trading fee is 1%, pons takes 30% of it
  (0.30%), and 0.70% is the creator side. With buybacks ON, half the
  creator side (0.35%) is spent buying the token back and 0.35% still
  arrives as cash. Buybacks do NOT zero our fees; they halve the cash leg.
- **`buybackEnabled` is ON.** Be precise about what it is and is not: it
  is not a trader reward, and nobody is airdropped anything. It converts
  half our cash into real bids, and bought tokens are locked and released
  over five years on a weighted clock, split creator/protocol — so we
  receive part of that leg back as tokens, and it cannot return as a
  sudden dump. If liquidity is too thin to buy back sensibly the buy is
  skipped and the money comes to us as cash anyway; the downside is
  bounded. NOTE the naming conflict to re-check on-chain before betting
  size: the struct field reads `buybackBurnBps` while the docs state
  bought-back tokens are explicitly not burned.
- **There is NO holder fee-sharing feature.** Searched the v2 docs: zero
  hits for holder rewards or distributions. The only mechanism that moves
  the creator's fees to a community is a Community Takeover, which
  redirects fees and the buyback share when a creator walks away — an
  abandonment path, not a launch setting. Real holder sharing would mean
  pointing `creatorFeeRecipient` at a distributor we write ourselves,
  which custodies other people's money and therefore needs a third-party
  audit before deploy. Do not promise holder rewards we have not built.
- **The cost is real and we take it anyway.** 0.35% instead of 0.70% of
  volume. A launch nobody wants to buy earns 100% of nothing, and terms
  written for the buyer are how volume shows up at all.
- **NEVER pitch the creator's cut.** The $LICK page shipped with "70% of
  every trade fee goes to the creator, on-chain" as its headline, repeated
  three times: an extraction brag aimed at the people being extracted
  from. Lead with what the buyer gets — no creator tax, fees recycled into
  locked buybacks, liquidity locked permanently, no mint, no blacklist.
  Our economics go in the copy as reassurance, never as the boast.

## Procedure
1. Verify the venue: confirm the live domain from the project's X account
   (@ponsdotfamily) and confirm real-time launch traffic on the explorer.
2. Derive and verify contracts on-chain (see clone warning). Read the
   factory's verified source on Blockscout; confirm the launch fee and fee
   split from the code, not a website.
3. Wallet ours + vaulted, funded with ETH for gas + launch fee + a modest
   dev-buy. Sign locally, always.
4. Launch with metadata via the verified factory; dev-buy in the same tx.
5. Verify: token + pool live on the explorer, metadata renders on the live
   site, fees accruing to OUR recipient. Book everything.

## Verification
Done means: venue and factory chain-verified, token + pool live on the
explorer, dev-buy landed inside the protection rules, fee accrual to our
recipient observed, launch booked (fee paid, addresses, txids). Any address
that came from a website and was not confirmed by live chain traffic is a
blocker, not a detail.
