The trading/investing lane's law: plan-before-entry (thesis, ladders, kill), kill-at-read, thesis revision on facts only, flagship never traded, allocation sized by judgment. Load BEFORE any discretionary position — entry, exit, re-plan, or portfolio review.
# trading_discipline — the plan trades, the moment doesn't

Discretionary investing (positions in assets the company did not create) is a
distinct lane from the flywheel (buybacks, launch dev-buys, LP). The flywheel
has its own skills; this one governs the portfolio.

## Hard rules
1. **The flagship stake is NEVER traded.** Permanent hold, no exceptions, not
   collateral, not inventory.
2. **No plan, no trade.** Every position enters with a full written plan
   recorded on the position row BEFORE the entry tx: thesis (falsifiable, one
   paragraph), entry price, size, laddered profit targets, kill condition.
3. **Ladders and kills execute when hit — they are never renegotiated in the
   moment.** Flexibility lives at the planning layer, never the execution
   layer. A hit level fires the same episode it is observed.
4. **A kill executes at the READ that produced it, not at the next scheduled
   checkpoint.** Checkpoints are the MINIMUM cadence for reading a position;
   they are never a reason to hold one that already meets its kill criteria.
   Paid for by a position whose falsifier was confirmed across two evaluator
   reads while execution waited for its scheduled checkpoint — the sell fired
   at a materially worse price than the first KILL read. Waiting for the next
   scheduled read IS renegotiating the kill in the moment. An evaluator or
   routine produces the verdict and the lane owner executes it, but the
   verdict binds the moment it is read, and a KILL left unexecuted past that
   read is itself a process violation.

5. **The entry gate and the kill are ONE definition.** Where the vet uses a
   momentum precondition to allow entry (token_vetting gate 9), the kill side
   is fixed at plan time from that same definition and its mirror: the
   short-window trajectory negative on two consecutive spaced reads = KILL at
   the second read, same episode per rule 4 — no volume corroboration, no
   both-legs interpretation. A position can never be entered on a momentum its
   kill cannot see. Paid for by a both-legs falsifier ("reversal WITH volume
   decay") that held a dead thesis for eight hours because volume never
   confirmed; the exit came materially worse than the plan's own trigger. Any
   change to the entry threshold re-derives the kill in the same edit.
## Sizing — the allocation is judgment, not a formula
The treasury is FIRST a development fund: fuel, launches, contracts, and new
opportunities always come before the portfolio, which gets only what the
company can afford to lose without slowing itself down. Start small; the
allocation grows only from booked realized profit — scale is earned, never
assumed from confidence. Working-capital floors always hold.

## Allocation is dynamic — re-derived, never frozen (founder rule 2026-08-31)
Fixed caps written once and quoted forever are the wrong shape; a cap frozen
in a note outlives the treasury math that justified it.
- **The working-capital floor is a FUNCTION, not a number**: derive it from
  real obligations — gas/fee runway, claim-cycle needs, enough liquid to
  execute every armed kill and exit at once. Recompute whenever the treasury
  or the obligations change, and record the current value WITH its
  derivation in OUR INSTANCE so sizing decisions load it.
- **Lane caps scale with deployable capital**: deployable = liquid treasury
  − floor − open committed positions. Lanes get shares of deployable, so
  capacity grows automatically with earnings and shrinks in drawdowns
  without a meeting.
- **Never cap the NUMBER of bets — cap total exposure.** The constraint is
  total at-risk versus deployable, never a slot count: however many
  independent bets deployable capital genuinely supports at sound individual
  sizes, run them. The portfolio brakes (loss-cluster pause, realized-loss
  breaker) sit on top of this, unchanged.

## Below the floor — survival mode, not a freeze (founder rule 2026-09-01)
The floor is a POSTURE TRIGGER, not a stop switch. A treasury below the
floor means the company works HARDER at making money, never that it stops
working — a company that dies holding its floor protected nothing.
- **Existence spending is exempt from the floor**: model fuel and the gas
  dust that revenue operations need keep flowing at any treasury level.
  These are the company's ability to think and act; the floor never
  outranks them.
- **Size against the OPERATIONAL floor, never the posture line** (founder
  2026-09-01, after the freeze). These are two different numbers and
  conflating them stops the company dead. The operational floor is gas
  obligations only: enough SOL to exit every open position and run a claim
  cycle, ~0.15 SOL at current fees. It is the ONLY term `deployable`
  subtracts:
  `deployable = MAX(0, treasury − operational_floor − open_committed)`.
  The posture line (the founder's 5.0 SOL) triggers this section and never
  enters the sizing formula. Do not pad the floor with a claim cycle's
  buyback leg (that is funded by the claim itself) or a reserved dev buy
  (an ambition, not an obligation). Sizing against the posture line is how
  4.717 SOL of live treasury computed to ZERO deployable and froze every
  lane, while fuel kept draining that same treasury — the money was not
  preserved, it was just spent on overhead instead of on attempts to earn.
  A company only stops when it can no longer pay the gas to sell what it
  holds. Until then it is still playing.
- **Capital gets rationed, ideas do not**: below the floor, spending needs
  an underwritten revenue premise — portfolio entries, infra upgrades and
  payouts wait, and deployable for premise-less bets is zero. But
  exploration INTENSIFIES: research, ideation, and premise-writing cost
  only fuel, and a treasury in drawdown is exactly when the company most
  needs new income ideas. Scan wider, score harder, write more premises —
  then spend only on the best-underwritten, nearest-revenue ones.
- **Attention reweights to making money**: below the floor the whole
  roster concentrates on booking income — claim cycles, fees owed, armed
  launches with fresh premises, paid endpoints, and NEW avenues that clear
  underwriting. Getting back above the floor IS the top-ranked objective
  while it is true.
- **Model economy rides the same posture**: when funds run low, mechanical
  and routine tasks (log reads, bookkeeping, monitors, formatting) move to
  the cheapest capable seat (deepseekv4flash-class), and the default seat
  (glm53flash-class) is reserved for planning, judgment, and thinking
  work. Burning premium tokens on mechanical work while low is the same
  mistake as discretionary spend below the floor.

## Profit ladders
Targets are planned at entry and taken mechanically in tranches (e.g. 25% at
+X%, 25% at +Y%, runner with a trailing rule) — sized so each tranche's swap
clears fees meaningfully. Selling a plan beats deciding at the top; the
ladder converts a guess about magnitude into guaranteed partial realization.

## Thesis revision — allowed on facts, banned on pain
A thesis may change when the FACTS change: a real event, new on-chain data, a
narrative confirmed dead or confirmed stronger. Revision = write the new fact
and a full new plan (targets AND kill re-derived) as a new version on the
position row. Two tests every revision must pass:
- "What changed besides the price?" must have a concrete answer an auditor
  would accept.
- Price moving against the position is NEVER the fact that loosens a kill.
  Kills fire; only genuine new information may re-plan — and if the new fact
  arrives while a kill level is already breached, the kill still wins.

## Review cadence
Each reflection: every open position gets checked against its own plan (level
hit? thesis fact intact?). A position whose thesis can no longer be stated in
one falsifiable paragraph is a position to exit — at that read, not the next.

## Mechanics (compose with existing skills)
- Mint allowlist discipline: on-chain verification before any new asset joins
  the allowlist; dust-test first buys; official swap paths only.
- Book both sides of every trade, sync positions to chain, one ledger line
  per txid (idempotence check first) — same as every treasury-moving action.
- First cycle is judged on PROCESS quality (plans written, ladders honored,
  books clean), not returns — prove the muscle before scaling it.

## OUR INSTANCE
Record here: the flagship mint (the never-traded stake), current allocation
cap and its derivation, open positions with plan versions, realized P&L to
date, and the working-capital floor.
