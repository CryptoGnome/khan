The safety pattern every tool must follow before it edits the live page or executes a one-shot money action: content-class gate, backup first, idempotent dry run, drift assertion, restore-and-residue test, full suite after. Load BEFORE writing or running any tool that writes to production or moves funds.

# safe_apply_to_live_tool — how a write-to-production tool is built

A one-shot action tool is written once and fired at the moment there is least
time to think. Everything that makes it safe has to be inside the tool, not in
the operator's head at 3am. The company executes its own one-shot actions —
there is no human on the trigger to catch a bad input.

## When to use
- Before writing, reviewing, or running any tool that edits the live page,
  flips public copy, applies a meta tag, or moves money (payout, swap, claim).
- NOT for read-only probes and NOT for tools that write only to scratch.

## Safety gates, in this order
1. **Content-class gate.** The tool refuses to write if the input is not the
   expected class — only meta tags when applying a meta tag, only the fee block
   when applying fee copy. Never let arbitrary content through a tool whose
   name implies a narrow job.
2. **Comment-strip before checks.** Strip HTML comments before any exact-match
   content check; comments carry stale text that both trips and masks greps.
3. **Backup first.** Write a timestamped copy of the target BEFORE the edit,
   named for the reason. Document the restore command in the tool's own help.
4. **Dry run must be idempotent.** On an already-current file the dry run
   reports zero changes; on the happy path it reports the exact expected count
   of replacements. A dry run that cannot tell those apart is not a dry run.
5. **Drift guard.** Assert the anchor text appears EXACTLY the expected number
   of times before replacing. Zero means the file moved under you; more than
   expected means you are about to edit the wrong occurrence too.
6. **Restore and residue check.** Test the restore path, then verify zero
   residue of the applied content; after a real apply, verify the old content
   is gone. Both directions, or neither is proven.
7. **Live verify.** After the write, byte-compare live against local. The edit
   is not landed until the server serves it.
8. **Full suite after.** Run the tool's own tests plus the page health check,
   structured-data validation, and the copy-figures audit. A green tool with a
   red page is a failed apply.

## Pitfalls
- Verifying a staged document against an apply tool by parsing the tool's
  source literals breaks on escaped quotes and nested string syntax. Compare
  the tool's DRY-RUN OUTPUT against the staged text instead — the output is
  the thing that will actually be written.
- Double-apply is the quiet corruption: the second run finds no anchor and
  either no-ops silently or appends. Make the refusal explicit and loud.
- Money paths add disclosure to the sequence: disclose BEFORE, execute, verify
  on-chain, disclose AFTER. The tool should refuse to execute if the before-
  disclosure is not recorded.

## Verification
The tool ships only when its own suite covers: no-op dry run, happy-path dry
run, wrong-content-class refusal, drift assertion, apply, restore, and
post-restore residue. Then the live apply is verified byte-for-byte against
the served response.

## OUR INSTANCE
Record here: the live page path and its backup convention, the names of the
health / structured-data / copy-audit tools, and the disclosure ledger call
that money-path applies must write.
