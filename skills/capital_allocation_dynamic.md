Load BEFORE any sizing decision — entry, dev buy, lane cap, or any dispatch that quotes a cap: the operational floor is gas obligations only, deployable is a formula not a number, and the working-capital line rations behaviour without ever freezing capital.

# capital_allocation_dynamic — how much the company may deploy right now

Every sizing mistake this framework exists to prevent had the same shape: a
number written down once and quoted for days after the treasury moved under
it. The formula is the law; the figures are a snapshot with an expiry.

## When to use
Before any decision that names a cap — a discretionary entry, a launch dev
buy, a lane budget, a dispatch that tells someone "up to X". Also load it the
moment a plan quotes a hardcoded cap or an old floor: replace the number with
a fresh derivation. This skill sizes; it never executes.

## The formula
```
operational_floor = cost to exit ALL open positions at once
                  + gas for one full income/claim cycle
deployable        = MAX(0, live_treasury - operational_floor - open_committed_at_cost)
lane_cap          = SHARE x deployable          # shares fixed per lane, e.g. 40/30/30
```

What the floor is NOT — both of these were once in it, and together with a
"floor is at least the working-capital line" rule they zeroed a treasury of
4.7 units while the company had every reason to trade:
- **No policy payout leg.** A buyback or revenue share owed out of a FUTURE
  income event is not an obligation the treasury pre-funds. Income events
  fund themselves.
- **No armed-but-unfired commitment.** A launch that has not fired has no
  capital obligation. It enters `open_committed` when the money is committed,
  not when the plan is written.

The floor is what it costs to get flat and to keep collecting income. Nothing
else is an obligation.

## The working-capital line is a posture trigger, not a wall
Below the founder's working-capital line the company **rations and works
harder** — smaller tickets, tighter kill clocks, a higher underwriting bar,
cheaper model seats on mechanical work. It does not freeze: deployable is the
formula's answer regardless of which side of the line the treasury sits on,
and every lane with an underwritten premise draws its share.

**The only hard stop is being unable to pay the gas to sell what you hold.**
If that day arrives the correct move is still to sell — gas is an existence
spend — never to sit on a position you cannot exit.

## Lean-times seat policy
While the treasury is under the line or the fuel projection is tight, move
mechanical work (scraping, formatting, auditing, clerical bookkeeping,
verification scripts) to the cheap model seats and reserve the expensive seat
for planning, judgment, review, and anything money-gating or public-facing.
The expensive seat only pays for itself on judgment; on mechanical work it is
several times the tokens for the same output. This is a posture, re-checked
every income cycle and every restart, and it lifts on its own when the
treasury recovers. Judge seats by the rated-quality tables, not by reputation.

## Procedure
1. Read the treasury balance LIVE from chain, by reference to the private
   endpoint — never from a note, and never from the founder's wallet (a
   wallet map pinned in memory prevents the confusion that cost two
   derivations in one day).
2. Compute `operational_floor` from real exit and cycle gas costs. Recompute
   whenever the position count or the fee regime changes.
3. `open_committed` = open at-risk positions at entry cost. Permanent or hold
   lanes — a flagship stake, an LP, identity NFTs — are not trading
   positions. Dormant off-chain balances are excluded until physically swept
   back to the main treasury.
4. `deployable = MAX(0, treasury - floor - open_committed)`.
5. Lane caps = shares of deployable. Re-derive at every income cycle, every
   restart, and every change in open positions.

## Main-treasury doctrine
One chain holds the main treasury and the book of record. Balances parked
elsewhere are excluded from floor, deployable and lane math until they are
physically swept home. Watchers may feed the math with data; every sweep is a
capital action that the CEO approves. Any bridge or swap episode ends with a
position re-sync and a books-vs-chain check at exit 0.

## Pitfalls
- A hardcoded cap goes stale the moment the treasury moves. Fix stale caps on
  sight rather than reasoning around them.
- Deployable subtracts open positions AT COST. Unrealized profit creates no
  capacity.
- The lane shares apply to whatever deployable exists. Small deployable means
  small lanes, never zero lanes.

## Verification
Print all four or the derivation is invalid: live treasury with its read,
the floor broken into named components, `open_committed` with each position
named, and deployable with each lane's share.

## OUR INSTANCE
Record here: the treasury wallet and how it is read, the current floor
figure with the date it was derived, the standing lane shares, the
working-capital line, and the superseded numbers that must not be re-cited.
