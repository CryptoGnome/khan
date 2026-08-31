A public page never hard-copies live data into its markup: every displayed number or list renders from a live source, with a dim placeholder when the feed is missing. Load BEFORE any draft, QA, deploy, or outsider review of a public page that shows figures, boards, or lists.

# no_baked_page_data — a stale board is worse than no board

A transparency page earns trust by being right at the moment someone reads it.
A number typed into the markup is right for one day and lies from then on, and
the lie is invisible: the page still looks live. This rule came out of a founder
QA pass that found a company board tab rendering from a JS array snapshotted at
build time, sitting next to genuinely live panels and the words "live from the
database".

## When to use
- Before drafting, QA-ing, deploying, or outsider-reviewing any public page
  section that displays a figure, a table, or a list that changes with the
  business.
- NOT for static policy copy (a buyback percentage, a canonical mint, a repo
  URL) and NOT for historical records that are correct precisely because they
  are frozen.

## The rule
Anything the page shows that changes with the business MUST arrive on a live
pipe and render in JS. There is exactly one such pipe — reuse it. Ours is a
stats event pushed onto a server-sent-events stream every few seconds and
written to the operational DB; whatever yours is, the board rides it rather
than getting its own endpoint the binary does not have.

Fail-soft: a missing field renders a dim placeholder (`--`, `board
unavailable`). NEVER a leftover snapshot, and never fallback text carrying a
real-looking amount — a plausible stale figure is the worst of both.

## Forbidden (the bug class)
- `window.PAGE_DATA = { objectives: [...], work: [...] }` — any JS array of
  live rows typed into the page.
- Copy claiming "live from the database" when the data was frozen at build.
- Reading the wrong database. A workspace/scratch DB will happily return rows
  with the right column names that are not the company's board. Wrong-DB is a
  bug, not a fallback.
- Baking a mutable figure into a share card or og:image — it is stale the day
  after the next cycle, and it is cached where you cannot fix it.

## Render contract
Update LEAF spans only, never a parent that contains children. Split sections
by lifecycle (active rows in one table, completed/dropped in another) — mixing
them is its own failure. Sort by the DB's own rank/id; render what the DB has
rather than curating a second list in JS. Truncate long notes; never dump full
internal plan text, secrets, or paths onto a public page.

## Order of work
Page render FIRST (as a draft), then the payload field, then QA both together.
Writing payload fields the page ignores and calling it shipped is the common
half-landing. Single writer: one draft file, one deploy.

## Verification
The pre-deploy check must carry a `no-baked-page-data` gate — a layout PASS
with a baked board is not a pass. Against the LIVE page only:
1. `grep` the served HTML for `PAGE_DATA` / `<field>: [` as a JS data array —
   any hit is a FAIL even if a freeze/hash tripwire matches.
2. Diff the rendered rows against the DB query the page claims to use.
3. Spot-check three numbers against the most recent live stats row.
Report; do not edit during a review.

## OUR INSTANCE
Record here: the page file, the stats-event field names your board rides, the
DB and table that is the real company board (and the decoy DB that is not),
and the current sanctioned page hash if you run a freeze tripwire.
