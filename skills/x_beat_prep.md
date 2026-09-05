Stage social standalones ahead of a posting window in a hashed on-disk file, then fire them behind freshness, spread, count and length gates re-run at fire time. Load BEFORE authoring a beat-prep staging file or firing any staged post from one.

# x_beat_prep — staged drafts are hypotheses until the gates re-run

Staging posts ahead of a window is cheap and good; firing a staged post without
re-running its gates is how a stale take goes out under the company name. The
file gates CONTENT. The gates at fire time gate REALITY.

## When to use
- Preparing drafts for a future posting window.
- Firing from an existing staging file.
- NOT for summoned replies — the reply wall rules govern those.

## The freshness gate (free, and it is a SELECTION gate)
Before staging and again before firing, sweep a free, dated, public news or
tech-forum API for beats above a points threshold in the last few hours. This
costs nothing and it RANKS: a genuinely fresh beat supersedes every evergreen
draft; if nothing is fresh, the strongest evergreen fires.

Two hard-won mechanics:
- **Build the query URL in the script, not the shell.** A bare-curl form with
  shell date interpolation mangled the URL into an HTTP 400 live. Do it in a
  language with a URL builder, URL-encode every comparison operator, and send a
  browser user-agent — bare requests get refused.
- **HTTP 200 with zero hits is a HEALTHY result**, not a broken query. It means
  nothing is fresh right now. Anything else (400, timeout, HTML) means the query
  shape is broken — fix it, never work around it.

## The single fire authority law
Exactly ONE named owner may fire a given staged beat, and the name lives on
disk — in the staging file's FIRE CONDITION section, or in a superseding fire
plan. If a dispatch handed you the beat but the file names somebody else, you
hold nothing: do not send until the file (or the superseding plan) carries your
name. When a dispatch moves a beat to a new owner, that owner is written into
the file BEFORE the send, not after.

Immediately before ANY send, re-read the free post ledger. If a row appeared
since your last read, another authority may already have fired — confirm the
post is not already live before adding a second. Two live authorities on one
beat is a coordination defect even when the platform absorbs it: a second send
burns the drafting and risks a visible duplicate wherever dedupe is slower. Two
authorities is not redundancy — if a dispatch and a staging file disagree about
who fires, the disagreement itself is the alarm, and it resolves to one name on
disk before anything sends. A held beat costs nothing; a duplicate costs
credibility. Written after a parallel fire landed three minutes behind the named
owner's post and was saved only by the platform's own duplicate rejection.

## Procedure
1. Scout beats from free sources only — never a paid social read for discovery.
2. Run the freshness gate. It both ranks the drafts and kills stale ones.
3. Author two or three drafts in the company voice, with no tickers, links,
   hashtags or self-promotion. Verify length programmatically before staging:
   the post tool REJECTS an over-length post rather than truncating, and the
   refusal still burns the drafting work. Leave headroom.
4. Write the staging file: per draft, its character count, a short content hash,
   the banned-word scan verdict, and the full text in a fenced block. Then a
   FIRE CONDITION section naming the priority order, every fire gate, and the
   single owner who may fire.
5. Verify the file on disk in the SAME session. A pointer without the text has
   burned a re-derive cycle before.
6. **At fire time re-run every gate**: freshness, the spread since the last
   post, today's post count read from the free budget check (another session
   may have posted without your coordination), length, banned-word scan, and a
   byte match of the chosen text against its staged hash, and the single fire
   authority check — you are the named owner, and the ledger read is clean in
   the same minute. Fire exactly one post unless the fire condition says
   otherwise.
7. Append to the spend log: pre-append before the paid call, then log the post
   id, cost and remaining balance after it.

## Pitfalls
- Waves rot. A draft staged hours ago is a hypothesis, not a plan.
- Tombstoned means tombstoned — do not paste a dead draft into the post tool
  because it "might still land". But confirm a beat is genuinely dead with a
  WIDER sweep; one momentary empty freshness window is not proof of death.
- A dispatch overrides the staging file's own timing notes. The file gates
  content, the dispatch gates timing.

## Verification
Done means: the staging file exists with a matching hash, the fired draft
byte-matches its recorded hash, the fire-time count read showed headroom, the single-authority check passed, and
the spend log carries the post line with its id. A fired post whose id was never
logged is unfinished.

## OUR INSTANCE
Record here: the account handle, the staging file path convention, the posting
window in UTC, the daily post ceiling, the spend log path, and which skills
govern pricing and voice.
