Getting a token listed and its metadata filled on the price aggregators and chart sites: which ones auto-index, which need a manual submission, the official submission URLs, and why the whole lane is founder-session-only from a datacenter. Load before any aggregator listing or token-info work.

# token_listing_submissions — which doors open, and which need a human

Chart sites index a new token on their own; the two big price aggregators do
not. Everything below the fold of that sentence is where the work is, and the
lane's hard ceiling is that every submission path is captcha-gated.

## When to use
Before any listing check, listing submission, or token-info metadata update on
a price aggregator or chart aggregator. NOT for the launch itself, and not for
the token's own site.

## Who auto-indexes and who does not
- **Auto-indexed, no action needed**: the chart/DEX aggregators that crawl
  launchpad pools. The token appears within minutes of the pool existing.
- **Manual submission required**: the two major price aggregators. They never
  auto-add a launchpad token.
- **Metadata is a separate lane from listing.** A chart aggregator will index
  the token with an empty profile — no logo, no categories, no socials, and a
  low trust score. Filling that profile is its own form.

## Status check first (read-only, keyless)
Every aggregator exposes a public search or token endpoint. Query it before
submitting anything: half of "we need to submit" turns out to be already
listed, and a submission on top of an existing listing is noise that slows the
real queue. Check the token profile endpoint too, and note exactly which fields
are already populated — re-entering a field that is already correct is how a
good profile gets bounced back into review.

## Official submission paths
Use the platform's OWN request form, never a third-party "listing service".
Each aggregator publishes one canonical form URL: a free tier measured in days
and a paid fast-track. The free tier is the default; a paid fast pass is a
capital decision and needs approval.

## The automation ceiling — this lane is human-session-only
Every submission path is gated by a bot-check widget that is enforced
SERVER-SIDE, and this was established the expensive way rather than assumed:
- The submission API contract can be read straight out of the site's production
  JS bundle — exact endpoint, exact envelope shape, exact field names. That
  work is worth doing once, because it tells you where the wall actually is.
- Correctly-shaped probes get PAST type validation and stop at the bot-check
  and one-time-code checks. That is the proof: the request shape was never the
  problem, and no amount of reshaping it will help.
- Headless browsers fail independently — the bot-check widget never renders,
  form dropdowns re-render and empty the fields, clicks intercept.
- The aggregators' support portals also return a datacenter 403 outright.

**Consequence: do not retry these probes.** The taxonomy is closed on evidence.
Hand the lane off with a copy-paste prefill pack; a real browser in a human
session passes the bot check passively. Re-deriving this each cycle is the
single most expensive way this lane wastes money.

## Pitfalls
- Re-derive every figure in the prefill pack at edit time from the books; a
  stale number here gets pasted into a live public submission.
- Never stamp a document with a future date — generate dates from the clock.
- Leave a social field blank rather than inventing an account for it.
- After any event that changes the numbers, refresh the prefill pack in the
  same episode.

## Verification
The status endpoint shows the token listed, or the submission has a ticket
reference. For a metadata update, re-read the token profile endpoint and name
which fields changed — a submitted form is not a landed change.

## OUR INSTANCE
Record here: the token name, symbol, mint/contract, pool address, creator or
treasury address, launch date, site and repo URLs, categories, the one-line
description, the logo path, and the path to the full submission pack.
