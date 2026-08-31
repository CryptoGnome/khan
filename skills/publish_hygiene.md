Pre-flight checklist before pushing ANY code to a public repo — engineer it so an abuse scanner reading it cold sees what a human sees. Load before every public push, repo creation, or PR to a third-party repo. Born from the 2026-08-31 GitHub flag: a benign health-checker read as malware.

# publish_hygiene — public code must look as honest as it is

A legitimate internal tool got the whole GitHub account restricted for
"known malware or scam pattern." Nothing in it was malicious. The problem
was the cluster: process scanning + crypto RPC + a deliberately hidden
env-var secret + a spoofed browser User-Agent, in one script, pushed by a
day-old automated account. Every abuse classifier on earth is trained to
fire on exactly that shape. Publishing is WANTED — the fix is engineering
public code so it cannot be mistaken for what it is not.

## The checklist (run before EVERY public push)

1. **Scanner-bait behaviors** — does the code do any of: enumerate
   processes (/proc, ps, tasklist), read credentials or env-var secrets,
   touch wallets/keys/seed phrases, hook input, take screenshots, spoof a
   browser User-Agent, download-and-execute, obfuscate strings? Each one
   is fine ONLY when it is the product's stated purpose, named in the
   README's first paragraph, with the why inline at the code site.
2. **Never combine them silently.** Two or more scanner-bait behaviors in
   one file is the infostealer signature. Split the tool, or document each
   behavior at the exact line with what it does and why.
3. **Ops tooling stays internal.** Published repos are PRODUCTS someone
   else would want. Health checkers, daemon watchdogs, claim-cycle
   scripts, anything wired to our own infrastructure: workspace only.
4. **Identify honestly.** No Mozilla/5.0 masquerade in published code —
   a UA that names the tool ("page-health/1.0") works nearly everywhere
   and reads clean.
5. **README before first push, not after.** A crypto-adjacent repo with
   no README on a young account is presumed scam. State what it is, what
   it touches, and what it never does, in the first screen.
6. **Young-account throttle.** While the account is new or recently
   restored, space out repo creation and third-party PRs; volume from a
   fresh automated account is itself a signal.

## If flagged anyway

Do not argue with the classifier and do not retry around restrictions:
alert the founder immediately with the exact repo named — appeals and
deletions are founder-in-browser actions. Keep the tool internally; a
public takedown loses nothing but the listing.
