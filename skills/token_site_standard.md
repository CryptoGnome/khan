The four standing rules every token launch site must pass — contract address above the fold, buy path with full disclosure, kept live after launch, and a design bar that rises each launch. Load BEFORE building or editing ANY token site, subdomain, or venue link field, on any chain.

# token_site_standard — the site is part of the launch

A token's site is the only place a buyer can verify what they are about to
buy. It is part of the launch, not an afterthought, and it does not stop being
work the day the token ships.

This came out of a launch whose site went live with NO contract address
anywhere on the page, and was never touched again after launch day. A buyer
landing there cannot verify what they are holding and cannot safely use the
page — so a site like that is WORSE than no site: it reads dead or
scam-adjacent to exactly the people it exists to convert.

## When to use
Before building or editing any page a buyer can reach from a token: a
subdomain, a launchpad site, the link fields in token metadata. Every venue,
every chain. Also before signing off a launch — all four rules are ship gates.

## THE FOUR RULES (all four pass before ship, and the page must KEEP passing)
1. **CONTRACT ADDRESS, IMMEDIATELY.** In full, copyable in one tap, ABOVE THE
   FOLD, from the moment the token exists. Verified character by character
   against on-chain state or the venue's official API — never transcribed
   from a notes file or a screenshot.
2. **BUY PATH + FULL DISCLOSURE.** A working buy link to the venue pair; the
   pairing asset named; what holders actually receive and by what mechanism;
   the dev holding with its exact percentage; the launch transaction hash.
   Every figure on the page matches on-chain truth at all times — a disclosure
   that drifts is a disclosure that lies.
3. **KEEP IT LIVE.** Update on graduation, on every fee claim, on every
   payout. A launch page that stops moving reads dead. Assign these updates to
   an owner as recurring lane work AT LAUNCH — never "later".
4. **RAISE THE BAR.** Study the winning launches' sites on the same venue and
   your own best work; each site beats the last. Render at desktop AND phone
   width (390px) and LOOK at both screenshots before calling anything done. A
   site you have not seen rendered is not done.

## Procedure
1. Load the frontend anti-slop skill and report its pre-flight count in the
   build report.
2. Build. Verify the contract address against the chain character by
   character; verify every disclosed figure against on-chain truth.
3. Screenshot desktop and 390px; view BOTH; fix overlaps, overflow, and
   escaped markup.
4. Report: pre-flight count, both screenshots, and the four rules each
   PASS/FAIL with the evidence behind each.
5. Stage the post-launch updates (graduation, fee claims, payouts) as lane
   work with a named owner before the launch is called done.

## Pitfalls
- Copying the flagship's branding onto an experiment token. Adapt the craft,
  keep the identities separate — an experiment that fails should not stain the
  main brand.
- Baking a mutable figure into the page markup or an og:image; see
  no_baked_page_data, which applies to token sites in full.
- Treating "site shipped" as terminal. Rule 3 makes the page a standing
  obligation, and an unowned obligation is an unmet one.

## OUR INSTANCE
Record here: the failure case that motivated the rebuild (which token, which
venue, what was missing), the house aesthetic reference to adapt from, the
subdomain pattern in use, and the evidence path where per-site screenshots and
verification notes are kept.
