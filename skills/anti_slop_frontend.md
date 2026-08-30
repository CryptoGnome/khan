Countable guardrails that keep khanbot.fun pages, share cards, and public copy from looking AI-templated — banned fonts/palettes/layouts, hero and CTA limits, the em-dash ban, and a pre-flight checklist a reviewer can literally count. Load before building or redesigning any public page section, landing view, share card, or marketing copy; also load for outsider-eyes site reviews. Not for the terminal-style live log itself (data-dense UI plays by its own rules).

# Anti-slop frontend — pages that don't look generated

AI-built pages share tells a visitor clocks in two seconds. Every rule here
is countable on the rendered page, so a reviewer (or a review routine) can
verify pass/fail without taste debates.

## When to use
- Building or changing any public-facing page section: homepage/landing
  views, tabs, share cards, coin pages, dev.to posts with layout.
- Running the site-outsider-review routine: this is the checklist.
- NOT for the live-log terminal view itself — density and monospace are its
  identity; judge it on legibility, not on these rules.

## Quick reference — the banlist (the AI tells)
- Fonts: not Inter/Roboto/system-default. Emphasize with weight/italic of
  the SAME family; never a stray serif word inside a sans headline.
- Color: ONE accent per page, saturation under 80%. No purple/blue glow
  gradients, no beige+brass "premium" default.
- Layout: no centered-hero-over-mesh-gradient, no three equal feature
  cards, no glassmorphism sprinkled everywhere.
- Copy: ban "Elevate / Seamless / Unleash / Next-Gen / Delve / In the world
  of". No invented fake-precise numbers — this site's numbers are REAL
  (treasury, claims, market cap); use those, dated. Sentence case headers.
- EM-DASH BAN: zero em-dashes in visible page copy. It is the #1 LLM tell.
  Period, comma, or restructure. One em-dash = pre-flight fail.

## Procedure
1. State a one-line Design Read before touching code: what kind of page,
   for whom, in what visual language. If the read genuinely forks, ask the
   dispatcher one question; otherwise declare and proceed.
2. Build within the countable limits:
   - Hero: at most 4 text elements (eyebrow OR brand strip, headline of
     2 lines max, subtext under 20 words, 1 primary + at most 1 secondary
     CTA). Everything else moves below the fold.
   - Eyebrow labels: at most one per three sections.
   - Image+text zigzag splits: never three in a row.
   - One accent color, one corner-radius scale, one theme per page — no
     light section sandwiched into a dark page.
   - No duplicate-intent CTAs ("Contact" + "Get in touch" = pick one).
3. Real images over fake ones: use the generate_image tool for hero and
   section art (it costs $0.01). No div-built fake screenshots, no
   hand-rolled decorative SVGs as the default.
3b. NO BAKED DATA. Any number or list that can change (treasury, board,
   claims, prices, team) renders from a live source — the stats/ledger SSE
   pipe or a file a routine regenerates — never hand-copied into markup. A
   transparency site showing stale data is lying with extra steps; the baked
   board tab and the baked USD panel were both this bug.
4. Ship the full states: loading (skeleton, not spinner), empty, error,
   hover/active feedback, and WCAG AA contrast on text and buttons.
5. Pre-flight count before reporting done — each of these is yes/no:
   Design Read stated · banned fonts/palettes absent · hero within 4 text
   elements · eyebrow and zigzag counts pass · one accent + one radius +
   one theme · zero visible em-dashes · CTAs deduplicated · real images ·
   zero baked changeable data (every live figure traced to its source) ·
   states + contrast shipped · copy free of the banned words.

## Pitfalls
- These rules are for PUBLIC marketing-facing surfaces; applying them to
  the terminal log view flattens its identity — wrong tool, wrong page.
- "Fixing" copy by swapping em-dashes for hyphens keeps the tell; rewrite
  the sentence.
- A page can pass every count and still be wrong for the audience — the
  Design Read exists so the counts serve a direction, not replace one.

## Verification
Run the pre-flight count on the RENDERED page (not the source) and put the
counts in your report. A failed count is a failed task, whatever the page
looks like to you.
