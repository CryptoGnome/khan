The cross-venue launch presentation law: NO token ships blank, on any launchpad, ever — metadata (image, description, links) is part of the product and is usually immutable, so it is right at launch or never. Load before ANY token launch on ANY venue, by ANY agent, probes included.

# launch_presentation — tokens ship dressed

Founder law (2026-09-01, after two blank launches on 4663). This binds on
EVERY launchpad — pump.fun, Pons, PAIR, Bags, whatever comes next — and on
every agent that fires a launch, not just the lane owner. It rides above
per-venue skills; a venue skill may add detail but never subtract from it.

## The law

- **Real launches ship FULLY dressed**: image/logo, a one-line thesis in
  the description, the X account link, and a site on one of our subdomains
  wherever the venue's metadata carries links. Users scanning listings
  filter for filled metadata with real links — a blank listing reads as a
  rug and forfeits the launch before the first candle.
- **Probe/test launches are never silent blanks**: name them as tests and
  put an honest description in the metadata ("infrastructure test, not for
  trading"). A blank token under our deployer is indistinguishable from an
  abandoned rug and stains every future launch from the same address.
- **Metadata is right at launch or never.** Assume immutability (proven on
  Pons: no setters exist). The pre-fire checklist therefore includes a
  METADATA POPULATED gate: image, description, links reviewed against the
  identity rules for that lane before the transaction is signed.
- **Where the venue metadata carries a website field, it is FILLED, and the
  site is LIVE before the fire** (founder 2026-09-01, obj39 successor prep:
  the package had logo/description/X link but left Socials.website empty).
  Build the site at sites/<ticker> so <ticker>.khanbot.fun renders, verify
  it, and pass the URL in the launch args — a write-once launch without the
  site link repeats the TAX mistake with extra steps.

## The platform-norm trap (why PAPER shipped blank)

"Every sampled launch ships empty URI" is an observation, NOT permission.
Other tokens looking abandoned is the opportunity: ours looking real is a
differentiator that costs minutes. Copying the floor's laziness forfeits
it. When a venue's metadata field is unclear, the move is to fill it
conservatively and honestly — never to leave it empty because others did.

## Scope

Per-venue skills (pumpfun_launch, pons_launchpad, and successors) carry
the mechanics; THIS law carries the standard. A launch dispatch that does
not name where the image, description, and links come from is not ready
to fire.
