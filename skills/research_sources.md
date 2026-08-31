Beat-first scouting map: which doors surface a story FIRST, in cheapest-first order, plus the date-verification rules and the prune rule that keeps the list honest. Load BEFORE any trend scan, opportunity scan, lane-beat verification, or "where do I look for what's hot" dispatch.

# research_sources — scout at the speed the beat moves

General web search is where a story goes to be confirmed, not where it breaks.
By the time a query returns it, the trade is priced. This map orders the doors
by who posts first, and the prune rule below is what stops it from silently
degrading into a list of sources nobody has ever learned anything from.

## When to use
- Before any trend scan, opportunity scan, or lane-beat verification.
- NOT for venue research on a launch, for trading premises, or for
  account-signup research — those have their own skills.

## Priority order (cheapest that answers wins)
1. **The venue's own API.** For a token lane, the launchpad's public API:
   top-by-market-cap AND newest-by-creation-time, plus an exact-ticker search
   for squatter probes. This is the floor read and nothing is faster.
2. **News RSS from a major engine.** Free, no auth, dated. Query PHRASING
   decides everything — compute age from the pubDate field, never eyeball it.
3. **Tech-forum search-by-date APIs** with a points threshold, for fresh
   technical beats; the same API at thread level for a story's live traction.
4. **Project changelogs and docs**, via `<domain>/llms.txt` then
   `llms-full.txt` then the docs tree. Always read these before coding against
   any API — research before build.
5. **On-chain data feeds** (aggregators, then a private RPC for owned assets).
   Aggregator APIs are flaky from a datacenter IP; retry once, then move on.
6. **General web search**, LOWEST priority. It backs up, it never leads.
7. **Paid social reads.** NEVER as a browsing tool — only when a decision
   hinges on reading one specific resource and no free route answers it.

## Procedure
1. Work the order above. Fetch the floor state and date-check every headline
   BEFORE scoring anything as a candidate.
2. Mark every single-outlet claim SINGLE-SOURCE-FAIL until a second
   independent source confirms it.
3. Record in the raw scan file WHICH source surfaced WHAT. That log is the
   evidence ledger the prune rule runs on.
4. When a beat is forum-only, set a mainstream-pickup clock (~12h) and re-run
   the specific news query at the deadline — that is what catches the crossing.

## Pitfalls (sharpest first)
- **A date without a year belongs to its document, not to today.** Resolve it
  from a commit date, a Last-Modified header, weekday arithmetic, or the
  article's own context — never "it must be this year". A fee-structure
  premise once shipped on an undated page whose facts had already changed.
- **Buried debunks surface through memory, not through fetching.** Recall the
  topic before committing to a premise; a recorded clone-site or stale-API
  flag inverts the verdict, and a fresh-looking fetch will not mention it.
- An empty RSS feed is NOT "no news" — re-run with different phrasing before
  calling a beat dead. The same query minutes apart can go from zero to ten.
- An exact-ticker search on a launchpad returns only small clones; the true
  lane leaders appear ONLY in the unfiltered top list. Check both, always.
- Trending and volume-sorted endpoints are the ones that get pulled or
  rate-limited first; a creation-time sort is the durable newest read.
- Consumer front-ends sit behind bot walls; use docs subdomains and public
  APIs, never the HTML.
- Never route RPC or model-API traffic through a scraping proxy.
- Never trust pre-event "successfully launched" copy.

## Prune rule (hard)
A source earns its place ONLY by having surfaced a beat FIRST at least once,
and its instance entry carries that evidence: the beat and the date. At every
review, cut any source that has never led — only confirmed what another door
already showed. A one-time wall does not prune a source; answering zero times
AND leading zero times does.

## Verification
Every listed source has (a) an evidence line naming a beat it surfaced first,
(b) a current answered/walled status from the most recent scan, and (c) no
entry present without evidence. Grep the raw scan files to confirm — those
logs are the ground truth, not this skill.

## OUR INSTANCE
Record here, as a table: source, exact endpoint or query form, status at last
check with its date, and the first-surfaced-beat evidence. Plus the house
rules that bind the map — the paid-read ceiling, which lanes are already
covered by staged kits, and any endpoint quirks (result counts that differ
from the requested limit, sorts that 400).
