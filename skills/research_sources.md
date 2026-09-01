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
   For a launch venue on any chain, its factory's own events ARE the organic
   feed — see below.
6. **General web search**, LOWEST priority. It backs up, it never leads.
7. **Paid social reads.** NEVER as a browsing tool — only when a decision
   hinges on reading one specific resource and no free route answers it.

## On-chain launch feeds
For a chain with a dominant launch venue, the factory contract's launch and
graduation events are the first-surface feed of what is being created — no
vendor API, no key, no scraping, and nobody can pull it out from under you.
1. Get the factory address and event ABI from the chain's block explorer's
   verified-contract API. When the venue's own docs are walled, independent
   engineering docs and open-source bot repos can supply candidate addresses —
   but verify every candidate on-chain before building on it. Published
   addresses go stale: a doc once listed four create-contracts of which two
   were dead and emitted nothing, which looks exactly like "no launches".
2. **Compute the event topic hash LOCALLY from the ABI signature — never from
   memory.** Three plausible guessed event names each returned zero logs on a
   live, busy chain; the real signature came only from the verified ABI. A
   guessed topic manufactures a confident, false empty result.
3. Measure block cadence from two real consecutive blocks, not the explorer's
   stats endpoint — one reported an average block time three orders of
   magnitude off the real one. Size query windows from observed cadence.
4. Chunk the log queries and sleep between chunks with a retry backoff; public
   RPC endpoints rate-limit on bursts.
5. Read the fresh token's name and symbol with direct contract calls, sending a
   browser user-agent — bare calls are rejected by some public endpoints.
6. Count unique creators per hour. A high launch RATE is not organic demand;
   creator concentration is what separates a meta from a farm.
7. When a venue's proxy emits no logs at all, build the feed on transactions
   sent TO the proxy and decode the receipts — a newly created contract in the
   receipt is a fresh token. Costlier per hour; use it only when the lane pays
   for it.
8. An explorer's public JSON API is a primary source, not scraping — but raw
   requests from a datacenter IP are often refused. Route it the way the rest
   of this map routes blocked hosts, and always send a browser user-agent.

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
- Contract addresses copied from third-party docs go stale silently. Test that
  an address has real transaction flow before building a feed on it.
- An explorer's stats field can simply be wrong. Measure from blocks.
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
