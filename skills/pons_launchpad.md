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
