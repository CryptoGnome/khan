Dedicated idea generation for new income: a standing cadence where a reasoning-tier seat hunts for what the company should build next, through explicit lenses, with every idea leaving as a written PREMISE for underwriting. Load when staffing ideation, when the scan pipeline runs dry or incremental, and whenever the treasury needs new income lanes.

# revenue_ideation — ideas are a produced good, not a byproduct

Scanning finds incremental opportunities; it does not originate. The scan
pipeline sweeps the same sources on the workhorse model and by construction
surfaces more of what it already knows. Idea generation is separate work,
done deliberately, on a seat built for thinking.

## The session (standing cadence, not a mood)

A planner on a REASONING-tier seat (ideation is thinking work — the one
class that justifies the expensive seat even in lean times) runs a session
on a standing cadence. Each session OPENS by reading three inputs, in order:
1. The scored idea list — never re-propose a live or already-killed idea.
2. The PREVIOUS session's output — every candidate must state what changed
   in the world since that session, or it is dead on arrival.
3. The lane kill log — kills are paid-for market data, and mining them is
   the dying-lane autopsy lens; it is an INPUT, not an option.

Then it works the lenses below and produces 2-5 written candidates. No
execution, no spend — paper only.

## INDEPENDENCE GATE (non-negotiable)
Ideation drifts. Left alone, sessions return thinly-disguised variants of the
lanes already running, because those are the lanes the session knows best —
this gate exists because independence had to be re-stated by hand in every
dispatch until it became part of the contract.
- Every premise names which LIVE lanes it shares dependencies with: same
  chain, same venue, same audience, same capital pool, same keystone event.
- A premise sharing ALL its dependencies with an existing lane is not a new
  lane; it is scope creep of that one. Hand it to that lane's owner or reject
  it in the session.
- A real new lane opens at least one dependency class the company does not
  already hold — a new venue type, a new payer type, or a new rail. Zero new
  dependency classes = not ideation output.
- The portfolio rule this serves: many hands on one thesis is still ONE bet.
  One wrong fact or one blocked keystone event zeroes the whole day.

## Lenses (work several, not one)

- **Copy-what-earns**: who is actually making money in this space right
  now — agents, small teams, protocols — and what is our version of it?
  Real revenue observed elsewhere beats any thought experiment.
- **Asset leverage**: what does the company uniquely already have — a
  treasury, an on-chain identity people can verify, a live site, an X
  account, a roster of agents, a machine-payments rail — and what should
  exist on top of those that doesn't yet?
- **Demand-side**: who would pay us TOMORROW, for what, at what price?
  Name the payer. An idea without a nameable payer is a hobby.
- **Picks and shovels**: when a platform or meta is hot, the tooling,
  data, and services around it earn steadier than bets placed on it.
- **Dying-lane autopsy**: for every lane killed by underwriting, ask what
  adjacent shape would have survived — kills are paid-for market data.

## X is the primary discovery feed

Now that the company has X API access, X is where crypto's builders
announce what they are building, what is earning, and what people are
paying for — long before it reaches aggregator sites. Use it as a
first-class ideation source: search the builder/dev corners (what shipped,
what's printing fees, what agent teams are launching), read the quote
threads where revenue screenshots circulate, and follow the money
language ("fees", "revenue", "paying users"), not the hype language.
Reads are cheap; ride the X budget ledger like every other call and keep
sessions to a few well-chosen searches, not a firehose. Order of
operations: FREE sources first (web_search, RSS, HN, on-chain data) for
anything that is not X-native, then X for what only X has — and batch the
X reads within one UTC day, because X deduplicates per-resource daily:
re-reads the same day are free, so a session that re-touches the morning's
posts costs nothing extra, while spreading the same reads across midnight
UTC pays twice.

## Output contract — every idea leaves as a PREMISE

Each candidate is written as a lane_underwriting-style PREMISE line:
expected income, cost to try, hit-rate x payoff x trial budget for
lottery shapes, the payer as a ROLE (never "the community"), time-to-first-
dollar, the falsifier, and the INDEPENDENCE LINE from the gate above. A
premise missing any one of these fields is not ready to leave the session.
Ideas enter the same scored pipeline
as scan candidates and compete on numbers. An idea that cannot state its
premise in numbers is not ready to leave the session.

## Worked examples — shapes that earn (founder, 2026-09-01)

Inspiration, NEVER recipes: copy the shape and the reasons it works, not
the instance. The market details here are dated — re-verify against live
X before building on any of them, because attention moves in days.

- **Pump.fun memes** earn when two things are true at once: genuinely
  funny, and fused to a LIVE trend (the trend makes it findable, the
  humor makes it spread — either alone fails). Presentation is part of
  the product: ALWAYS stand up a meme site on one of our subdomains and
  put that link plus the X account link in the token metadata — users
  scanning listings filter for filled-out metadata with real links, and
  a blank listing reads as a rug.
- **Games and programs with interesting tokenomics** — "ponzinomics",
  game theory, distribution mechanics that make holding/playing a game
  (X is full of these and they draw real volume). The mechanic IS the
  meme. Two hard rules: keep the code as SIMPLE as possible — a clever
  mechanic in complex code becomes unmanageable and dies of its own
  weight — and give any app or minigame its own well-designed site on a
  subdomain; the site is the product's face.
- **Degen tooling sold as an API** — the data crypto natives check
  before aping, packaged: e.g. a bubble-map competitor where an address
  or token in yields launch data, bundling analysis, sniper detection,
  holder concentration (top-10/top-20 share, supply distribution).
  Monetize per-call via x402, or launch a token that IS the access key
  (pay in the token to use the product — the product gives the token a
  reason to exist). We already run the x402 rail; tools like this are
  its natural payload.
- **Smart contracts people will use, on cheap chains with attention** —
  not Ethereum mainnet (fees kill usage). New sidechains are the
  opening: Robinhood Chain has Uniswap v4 hooks live and hot right now;
  Base is cheap but attention-dry at the moment. The chain landscape is
  the fastest-staling fact in this file — treat these as the base
  directive and update the read from X every session.
- **Solana programs** — the same product shapes as EVM contracts but a
  different machine, and it is OUR home turf: fees are near zero,
  attention is permanent, and the treasury, identity, and tooling
  already live there. The account model, PDAs, and CPI composability
  enable mechanics EVM does poorly — programs that hold and route state
  cheaply per user, fee-split/escrow programs other launches plug into,
  on-chain games where every move is an affordable transaction, token
  mechanics wired straight into transfers. But programs are paid for BY
  THE BYTE: deploy rent scales with binary size, so every instruction
  and dependency in the program is SOL locked up at launch — write the
  MOST MINIMAL program that does the job (strip unused Anchor features,
  avoid fat dependencies, keep instructions few), sized small for rent
  without ever trading away security: minimal and secure, never minimal
  instead of secure. The simplicity rule binds double anyway (deployed
  program bugs are drained treasuries), and NOTHING deploys to mainnet
  without a full devnet/testnet pass first — a testnet system for every
  program, every time, then dust-scale on mainnet before real size.

## Quality bar

- Prefer ideas that earn while we sleep (endpoints, fees, standing
  services) over ideas that need an operator per dollar.
- Prefer verifiable-on-chain revenue over promised revenue.
- One ambitious candidate per session minimum: something that would move
  the treasury 10%+, not only safe dust-scale plays. Ambition is cheap on
  paper — underwriting is where it gets disciplined.
