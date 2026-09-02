Verify the world, not the self-report — for the claims that can hurt: money moved, something public changed, something irreversible happened. Everything else is rated on the evidence the report carries (txid, row id, file path + hash, URL). Load whenever reviewing a report, rating work, or running a review routine.

# Verify the world, not the self-report

A report is a claim, not evidence. An employee that hit an error can still
write "done, deployed, working" — not from malice, but because models
summarize intent as outcome. A rating based on the report alone trains the
whole company on fiction: prompt evolution keeps what scored well, so an
unverified 5 teaches every future employee that confident prose beats
finished work.

## When to check the world at all

Only when the load-bearing claim is one of these:

- money moved or was committed (a send, swap, launch, claim, payout, top-up);
- something PUBLIC changed (the live page, a post, a listing, a repo push);
- something irreversible happened (a kill-exit, a deletion, a contract call).

For every other report — research, drafts, evidence files, scans, sweeps,
status reads — do NOT spend a call re-checking it. The report must carry its
own evidence (row ids, file paths with sizes or hashes, txids, URLs); rate on
that. A report with no checkable artifact caps at 3 and is told so. Founder
rule 2026-09-02: on 2026-09-01, 218 of 269 ratings were preceded by a CEO
spot-check and half of all dispatches were checks of earlier work; one
launch shipped. Verification had become the product.

## The method (for the three cases above)

Pick the ONE most load-bearing claim — the thing that, if false, makes the
report worthless — and check it against the world, not the transcript:

- "I wrote/updated file X" → read_file X. Does it contain what the report says?
- "The page is live/fixed" → web_fetch the actual URL. Is the change there?
- "I recorded/inserted the data" → sql a SELECT for the row.
- "The script works now" → shell: run it once. Exit clean?
- "I posted it" → x_read or web_fetch the post.

One claim, one check, and only for money / public / irreversible. Never
dispatch an employee to re-verify another employee's report: the binary
refuses the fourth check-class dispatch in a row on an objective with nothing
built between, and the same task shape three times in a day is refused as
routine work — write the script and add_routine it.

## Rating rules

- Verified claim holds → rate on the work's actual quality.
- Claim contradicted by the world → rate 1 or 2, whatever the prose sounded
  like, and say exactly what you checked and what you found in the rating
  note — that note is what teaches the prompt evolution WHY.
- Claim unverifiable (no file, no URL, no row, nothing runnable) → that is
  itself a finding: cap at 3 and tell the employee reports must point at
  checkable artifacts.

## Gotchas

- Check the world AFTER reading the report, not before — you are verifying
  their claim, not doing their task.
- A stale cache can make a true claim look false (page not refreshed): if a
  web check contradicts a fresh-deploy claim, re-fetch once before rating 1.
- Review routines: this applies doubly — a scheduled reviewer that rubber-
  stamps self-reports is worse than no reviewer, because it launders them.
