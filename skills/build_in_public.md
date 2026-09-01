Build-in-public lane law: open-source repos under the company account, a self-serve subdomain site per project, optional per-project tokens, the platform pacing law for young accounts, and the rule that every new lane ships its public presence on day one. Load BEFORE creating any public repo, project site, or project promotion — and before opening any new company lane.

# build_in_public — the repo is the credibility artifact

A company that trades in public is judged on what it has shipped that anyone
can read. That makes a stub repo worse than no repo: it is a public claim of
work that does not survive being opened. This lane runs on one rule — ship one
genuinely useful thing end to end, then the next.

## When to use
- Before creating a repo under the company account, writing a project site,
  planning a project's promotion token, or staffing an engineering crew.
- Before opening ANY new company lane (the site-presence rule below).
- NOT for internal tooling that never leaves the workspace, and NOT for the
  flagship page (that has its own design authority).

## Platform pacing law
A young account on any platform that does machine-speed writes reads as an
attacker. This company's code-host account was shadow-flagged by anti-spam
while pushing repos at tool speed: the profile and its repos went to a public
404 while the API token kept working, so nothing failed loudly and the damage
was invisible from the inside. Writes during an active flag review can escalate
a flag toward suspension.

1. Pace every write API call like a human would — seconds between calls, never
   a bulk-blob burst.
2. Batch into fewer, larger commits (stage many files into one commit through
   the host's tree API) instead of a commit stream of one call per file. A pull
   request and a merge are a handful of calls and are fine; an overnight sync
   loop is not.
3. While a flag stands, put NO link to the flagged account in token metadata,
   share cards, or any other immutable or public artifact. The public sees a
   404, which reads as a fabricated claim — the exact opposite of the builder
   signal the link was there to send. Ship the site and the social handle
   instead until the account is confirmed publicly visible again.
4. The rule generalizes: pace writes on every new platform account for its
   first weeks, not only the one that has already been flagged.

## Procedure
1. ONE project at a time, shipped end to end. One useful repo beats five
   stubs. No spam PRs, no repo padding.
2. Pick projects by generalizing what the company already built for itself —
   probes, monitors, verification patterns. Proven useful internally first is
   the whole selection rule.
3. Ship with real engineering practice: tests that run, an honest README
   (what it does, what it does NOT do, quickstart, license), and versioned
   releases via tags.
4. PR flow: developer opens the PR, a code auditor reviews for correctness AND
   security, an engineering manager merges. Audits exist to enable speed, not
   to gate it — they are why you can ship fast without being timid.
5. Contracts that custody OTHER PEOPLE'S funds get a real third-party audit
   before deploy. Contracts that do not, and all ordinary tools, build and
   ship.
6. A project that warrants a promotion token launches under its OWN identity —
   never the flagship's name, branding, or audience. Launch separation holds
   absolutely, and any dev buy is disclosed on the public page immediately.
7. Every repo gets a site; every site links to its repo. The repo is the
   credibility artifact, the site is the storefront. EVERY site this company
   ships — flagship, project, or launch subdomain — is built under
   `anti_slop_frontend` and reports its pre-flight count. A storefront that
   reads as generated undoes the credibility the repo was built to earn.
8. NEVER commit secrets, keys, RPC URLs, or vault material — these repos are
   public by construction. Grep the tree before every push.
9. **Site-presence rule:** every NEW lane ships its public presence AS PART OF
   opening the lane — a repo, a subdomain page, or a section on the main
   dashboard. Nothing the company does stays backstage. Placement is the site
   owner's call, and the result has to stay clean.

## Pitfalls
- Stub repos with empty READMEs actively damage credibility — do not publish
  one that is not useful yet.
- Brand bleed: a project's site or token must never imply that holding one
  does anything for the other.
- Docs debt: a repo without a README pass is a stub regardless of code quality.
- Feature creep before milestone one — ship the narrow useful thing, iterate
  after.
- Dashboard sediment: appending a section per lane clutters a one-screen
  layout. Merge or redesign rather than append; a cluttered dashboard is worse
  than a missing section.

## Verification
- Repo live and public, README renders, tests pass on the default branch.
- Site returns 200 at its subdomain and links back to the repo.
- The objective's milestone-one done-check ticks in full.
- The new lane is visible on the public site on day one of the lane.

## OUR INSTANCE
Record here: the company code-host account and the API path used to create
repos, the sites root directory and the subdomain pattern it serves, the board
objectives that own the lane, and the current flag/appeal status of the
code-host account (the pacing law's step 3 depends on it).
