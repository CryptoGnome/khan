Prove a public-page refresh actually landed: deployed bytes identical to the local file, every new figure present, every superseded figure gone, health checks green, a rollback backup on disk. Load AFTER any page change that touches money figures — treasury, fees, claims, stake, payouts.

# page_refresh_verification — a refresh is not done until it is proven

"The page was updated" is a claim, not a fact. The failure this catches is not
a broken deploy — it is a partial one: the new figure lands, the old figure
survives three paragraphs down, and the page now states two different treasury
balances. Rate the refresh against this check, never against the task text.

## When to use
- After ANY page change touching treasury, fee, claim, stake, or payout
  numbers, whether from a cycle refresh or a copy-hygiene pass.
- NOT for a pure layout or styling change with no figures in the diff.

## Procedure
1. **Byte-identical deploy.** Fetch the live URL and compare a hash of the
   response body against the local file the server is supposed to serve. A
   mismatch means either the deploy is broken or a writer is mid-edit — under
   a single-writer rule, wait for their report and re-check rather than
   trusting any number on the page.
2. **Grep for the new figures AND the old ones.** Two greps, not one:
   ```
   grep -oE "<new-fig-1>|<new-fig-2>|<new-txid>" page.html | sort | uniq -c   # expect hits
   grep -oE "<old-fig-1>|<old-fig-2>|<old-txid>" page.html | sort | uniq -c   # expect EMPTY
   ```
   Count every representation of the same number — a figure typically appears
   as a raw amount, a rounded abbreviation, and a percentage, and refreshers
   routinely update one and miss the other two.
3. **Run the health checks.** Site returns 200 with its content markers, the
   live stats feed is fresh (seconds, not minutes), the DB figure agrees with
   the on-chain value within tolerance, exactly the expected daemons are
   running and no retired one is, no null prices. Partial green is red.
4. **Confirm a backup exists.** A timestamped copy of the previous version,
   named for the reason (`page.html.bak_<cycle>`), written BEFORE the edit.
   The backup chain is the entire rollback story; a deploy without one is not
   verified regardless of how clean the greps are.

## Pitfalls
- The stale set advances every cycle. After each refresh, the figures you just
  wrote become the next run's stale list — keep the audit's stale set updated,
  or the grep silently passes on numbers nobody is looking for any more.
- Stale figures hide in rounded form and in comments; strip HTML comments
  before exact-match checks, since a comment can carry old text that trips or
  masks a grep.
- The live page may briefly serve the old file while a writer is mid-edit —
  a single early fetch can produce a confident wrong verdict.

## Verification
A refresh is PASS only with all four: byte-identical deploy, every new figure
present, zero stale hits, health checks green with a backup on disk. Anything
less is reported as a partial landing with the specific missing item named.

## OUR INSTANCE
Record here: the live URL and the local file it serves, the health-check and
copy-audit tool names, the backup path convention, and the current cycle's
new/stale figure sets.
