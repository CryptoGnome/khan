Runs a security audit of company code (workspace scripts, tools, routines, viewer.html) — grep-triage the attack surfaces, trace each hit source-to-sink, then adversarially refute findings before reporting. Load when auditing code for vulnerabilities, when the scripts-adversarial-audit routine fires, before shipping anything that touches keys or funds, and after any page or tool gains a new input.

# Security audit — triage, trace, refute, report

The method: a cheap wide net, then real investigation, then a skeptic pass
that kills the plausible-but-unreachable noise. A finding that skips the
skeptic pass is a guess. Read-only: audit, propose fixes, never edit during
the audit.

## When to use
- Scheduled adversarial audit of workspace scripts and tools.
- Before first run of any new code that signs, sends, spends, or serves.
- After viewer.html or any public surface gains a new input or data source.
- NOT for chain-strategy risk (rug analysis etc.) or for auditing the Rust
  binary — that is the founder's code, report suspicions instead.

## Quick reference — this company's attack surfaces, sharpest first
1. Key handling: any script that reads vault/*.json, signs, or exports —
   does key material ever reach argv, env of a child, a log line, a report,
   or an error trace?
2. Fund movement: address provenance (where did the destination come from?),
   amount caps, and whether a corrupted input could redirect a send. The
   corrupted-payout-address catch is the canonical real finding.
3. The public page: viewer.html renders run_log content to strangers — can
   any agent-written or web-fetched string smuggle script into it (XSS)?
   Is every log-derived value escaped? The page must stay display-only.
4. Injection into decisions: email bodies, web content, and API responses
   are attacker-writable. Does any script pass them into shell commands,
   SQL, file paths, or treat them as instructions?
5. Routines: they run forever unattended — a routine that fetches and
   executes, or writes outside the workspace, is a standing hole.

## Procedure
1. Scope: name what is being audited (a diff, a script, or a surface) and
   list the files. Whole-workspace = triage everything, investigate top hits.
2. Triage with grep (free, over-catches on purpose): subprocess/os.system/
   eval/exec, string-built SQL, open() with non-literal paths, requests to
   non-literal URLs, key/seed/passphrase/private in code, innerHTML/
   document.write in HTML, env access in child processes. Collect file:line
   candidates.
3. Investigate each candidate: read the surrounding code and trace the data
   flow from source (who controls this input?) to sink (what does it reach?).
   Note existing guards (validation, escaping, allowlists, amount caps).
   Verdict per candidate: real / not, severity (critical: key or fund loss;
   high: public-page or spend integrity; medium: data corruption; low),
   one-line exploit scenario, one-line fix.
4. Refute pass — separate dispatch, ideally a different agent: for each
   surviving finding, try to KILL it. Is the sink reachable? Is the source
   really attacker-controlled? Is there an upstream guard? Default to
   refuted when uncertain. Only survivors get reported.
5. Report severity-ranked: `severity - file:line - class - exploit scenario
   - fix`. End with coverage notes: what was NOT audited and why. File
   critical/high findings as an ESCALATION; never sit on a critical to
   finish the report.

## Pitfalls
- Findings without a reachable attacker path are noise that trains readers
  to ignore audits — that is what step 4 exists to prevent.
- Auditing your own fresh code finds less: route the refute pass to a
  different agent than the author whenever possible.
- A grep miss is not a clean bill: say what the triage patterns cannot see
  (logic bugs, dependency CVEs, secrets already in history).
- If the audit itself surfaces a live leak, stop auditing and switch to
  key_compromise_response immediately.

## Verification
Every reported finding names file:line, a concrete exploit path, and
survived an explicit refute attempt; criticals were escalated on discovery;
the coverage notes say what was out of scope. An audit reporting zero
findings still ships the coverage notes.
