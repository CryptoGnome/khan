The house standard for writing and updating skills with create_skill — how to write a description that actually triggers, the required section order, and the quality bar. Load whenever creating a new skill, rewriting an existing one, or when a skill failed to prevent a repeat mistake.

# Writing skills that get used

A skill is paid for twice: once in the index every agent reads every turn,
and again when loaded. It earns that only if it triggers at the right moment
and changes what the reader does. Most skill failures are description
failures — the body was fine, the index line never fired.

## When to use
- Before any create_skill call (new skill or new version).
- After an incident a skill should have prevented but did not: fix the skill
  the same day, and check whether its description was the failure.
- NOT for one-off notes — those go in remember/playbooks, not the index.

## The description (the index line) — this is the trigger
Write it as: what it does + WHEN to load it + the concrete nouns and verbs a
task would contain. Third person, no "helps with".
- BAD:  "Helps with launches."
- GOOD: "Launch a pump.fun token with a compliant dev buy via the official
  API. Load BEFORE any token launch, create call, or dev-buy sizing."
An agent scanning 30 index lines must know from this line alone whether to
load it. If a skill keeps being ignored, rewrite this line first.

## Required section order in the content
1. **When to use** — trigger conditions, plus when NOT to use it.
2. **Quick reference** — the exact commands/endpoints/paths, copyable.
3. **Procedure** — numbered steps, common path first, edge cases last.
4. **Pitfalls** — known failure modes, each one sentence, sharpest first.
5. **Verification** — what "done" verifiably means; a check the reader can
   run. Work without its check is unfinished.

## Freedom scales with risk
- Judgment work (copy, research, design): give principles, not steps.
- Money paths, launches, key handling, deletes: LOW FREEDOM — exact numbered
  steps, "do not improvise", explicit stop-and-report conditions.

## Portability — a skill may outlive this instance
Skills get harvested back into the core repo as seeds for fresh installs, so
write the body for any company running this binary, not just ours.
- The procedure teaches the method and the WHY — name the incident that
  motivated a rule (reasons transfer; bare rules get rationalized past).
- Company-specific facts (our addresses, account IDs, file paths under
  /data, current balances/prices) live in one **## OUR INSTANCE** section at
  the end — never woven through the procedure. Harvest = drop that section.
- No secrets, vault paths, or proxy details anywhere in a skill, ever.

## Quality bar
- Only what the reader does not already know. No explaining what a wallet or
  a bridge is; no restating the mandate.
- One default per choice (one aggregator, one endpoint) — alternatives only
  with the condition that switches to them.
- Concrete beats abstract: real commands, real addresses, real prices with a
  date, plus where to fetch the live value.
- Under ~100 lines. Longer means it is two skills or contains chatter.
- Same name = new version; put WHY in the reason field so rollback_skill has
  something to judge.

## Verification
Read only your new description next to the other index lines: would a busy
agent load it at the right moment and never at the wrong one? Then read the
body asking "what would the reader do differently?" — any paragraph that
changes nothing gets cut.
