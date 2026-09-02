Which database each SQL tool talks to, the one-statement rule, the column names agents keep guessing wrong, and the raw-base-units trap on position sizes. Load BEFORE any sql or db-query call — it turns the recurring "no such column / no such table" false-failures into a lookup.

# workspace_db_schema — the tool is not broken, the column name is wrong

Two databases sit behind two different tools, and a `no such table` from the
wrong one looks exactly like infrastructure being down. Every rule below was
paid for by an agent reporting a healthy system as broken, or by a number being
read off by a factor of a million.

## When to use
Before any SQL call against the workspace database or the binary's own
database, and immediately after any `no such column` / `no such table` result.
NOT a substitute for `PRAGMA table_info(<table>)` on a table this skill does
not list — PRAGMA first, always.

## THE ONE-STATEMENT RULE
The `sql` tool executes a multi-statement string but returns **only the first
result set** — silently, with no error and no hint that more results existed. A
second `SELECT` after a `;` runs and its rows vanish.
- Symptom: "the query didn't return" a row you know exists.
- Rule: ONE statement per call, always. Two tables means two calls (they can go
  in one parallel block). Never debug a missing row that was really a swallowed
  second result set.

## Two databases, two tools
- The `sql` tool talks ONLY to the workspace database (the company's own books:
  P&L, positions, ledger, local mirrors).
- The binary's operational database (run_log, agents, prompts, routines,
  ratings, model_calls, tool_calls, episodes, messages, memories, objectives,
  skill_defs, skill_loads, routine_alerts, kv) is read-only through the
  db-query tool. `no such table: routines` from `sql` is the WRONG TOOL, not a
  missing table.
- The operational DB has no `stats` table. Live site stats are `run_log` rows
  with `event='stats'`; the payload column is `detail` (JSON), not `note` or
  `payload_json`.

## The column-name confusions that keep recurring
- **`kind` vs `category`.** The public disclosure ledger uses `kind`; the P&L
  table uses `category`. The two tables look alike and agents swap them
  constantly. Write disclosures through the ledger tool; read P&L with
  `category`.
- **The ledger's kind vocabulary is closed** (buy, sell, claim, payout,
  deposit, withdraw, proposal, fee, other) and REJECTS P&L category words like
  `expense` or `ai_spend`. An expense that is not a buy/sell/fee discloses as
  `kind='other'` with the nature in the note, while the P&L row carries the
  real category. A rejected kind is a vocabulary error, not a broken tool.
- **The ledger has no `id` column** — order by `rowid`.
- **The local objectives mirror is not the board.** It carries only the few
  columns it declares; rank, owner, note and blocked_by live in the objectives
  TOOL. Selecting board columns from the mirror is the recurring
  `no such column: rank` failure.
- Time columns are not uniformly `ts`: some tables use `added`, `detected_ts`,
  `closed_ts`. PRAGMA before assuming.
- Most tables carry txids inside `note`, not in a `txid` column. Search them
  with the lookup tool, not `SELECT ... txid`.

## THE UNITS RULE — position size is RAW BASE UNITS
Launch dev-bag rows store `size` as the raw integer base-unit balance, NOT a
human token count. With a 6-decimal token the real position is `size / 10^6`.
Multiplying raw size by price once reported every open bag at roughly −18% when
each was within ±1% of cost.
- Rule of thumb: if a position's implied value looks absurd (a five-cent buy
  "worth" thousands), suspect units before suspecting the market. Cross-check
  against the mint's decimals or a position-value tool that returns both the
  raw balance and the real value.
- Same discipline when writing: record the raw on-chain amount in `size` so it
  matches a balance read, and put the human count in the note.

## Hard rules
1. Unknown table → PRAGMA first, through the right tool.
2. A `no such column` / `no such table` is YOUR query. Do not retry the same
   guess and do not report it as infrastructure down.
3. Row lookups by txid go through the lookup tool; live balances come from the
   chain tools, never from a snapshot table.
4. Snapshot tables are snapshots. Any episode that moves treasury funds
   re-syncs the touched row to on-chain truth before it finishes, and books
   both legs.
5. One statement per call.
