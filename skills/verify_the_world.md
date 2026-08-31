Verify the world, not the self-report: before rating any delegated report 4 or 5, spot-check one load-bearing claim against reality — read the file it says it wrote, fetch the page it says is live, query the row it says it inserted. Load whenever reviewing a report, rating work, or running a review routine.

# Verify the world, not the self-report

A report is a claim, not evidence. An employee that hit an error can still
write "done, deployed, working" — not from malice, but because models
summarize intent as outcome. A rating based on the report alone trains the
whole company on fiction: prompt evolution keeps what scored well, so an
unverified 5 teaches every future employee that confident prose beats
finished work.

## The method

Before rating a report 4 or 5, pick the ONE most load-bearing claim in it —
the thing that, if false, makes the report worthless — and check it against
the world, not the transcript:

- "I wrote/updated file X" → read_file X. Does it contain what the report says?
- "The page is live/fixed" → web_fetch the actual URL. Is the change there?
- "I recorded/inserted the data" → sql a SELECT for the row.
- "The script works now" → shell: run it once. Exit clean?
- "I posted it" → x_read or web_fetch the post.

One claim, one check. Verifying everything would double the cost of every
dispatch; verifying the load-bearing claim catches nearly all fiction for
one cheap tool call.

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
