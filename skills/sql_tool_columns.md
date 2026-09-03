Load BEFORE any database query: which tool sees which database, the PRAGMA-first rule that stops schema guesses, and the failure signatures that mean "wrong tool" rather than "missing table".

# sql_tool_columns — pick the tool, then read the schema, then query

Two databases, two tools, and neither tool can see the other's tables. The
company database (the binary's own: run history, agent records, skill
definitions, routine state) is reachable only through the read-only company
query tool. The workspace database (the books: ledger, pnl, positions,
objectives, and whatever the company has added) is reachable only through the
workspace sql tool, which is hardwired to that file. Nearly every query
failure in a bad window is one of two things: the wrong tool, or a column
name recalled from memory.

## When to use
Before any query against either database, from any caller — a tool call, a
routine script, or a shell sqlite3 invocation.

## The wrong-tool signature
`no such table: <a table you are sure exists>` almost never means the table is
missing. It means the query went to the other database. Switch tools. NEVER
"fix" it by creating the table in the database that answered — that manufactures
a second, empty, divergent copy of real company state.

Related signature: when the workspace sql tool errors, it dumps that
database's full schema. A schema dump in an error message means "wrong tool or
wrong table" and nothing more; it is not an invitation to redesign anything.

## The PRAGMA-first rule (the important part)
**Before any query against a table you have not queried in THIS episode, run
`PRAGMA table_info(<table>)` — or `SELECT * FROM <table> LIMIT 1` — and read
the real columns.** Never assume a column name from memory, not even one you
are certain of.

The rule is written in its strongest form because every repeat offender was a
confident guess: a log table queried by a foreign-key column it does not have
(six failures in a single window), an ideas table queried by the obvious names
for its title and timestamp columns when it uses neither. Each one cost
several failed calls; each would have been prevented by one PRAGMA.

Memory of a schema does not survive across episodes. The PRAGMA does. Any
list of known column confusions is a record of past guesses — a warning, never
a substitute for reading the schema.

## Pitfalls
- Two tables that both carry a classification column rarely name it the same
  thing. Confirm per table rather than reusing the name that worked next door.
- A transaction id is often not a column at all — it may live inside a free-text
  note, findable only with a LIKE against a fragment. Check the schema before
  concluding a field is missing from the data.
- A table without an explicit `id` still has `rowid`; reach for it rather than
  inventing a key.
- SQLite rejects `ORDER BY` / `LIMIT` inside the ARMS of a compound (`UNION`)
  select — the error names the UNION, not the arm, so it reads as a mystery.
  The intuitive "latest row per cohort" shape is invalid; wrap each arm as its
  own parenthesized subquery and put a single `ORDER BY` after the union, or
  simply run one query per cohort and merge the rows in your report. This one
  recurs: workers reach for the invalid shape again months after it was
  documented, because it is what every other dialect allows.
- Snapshot tables (balances, positions) are stale by construction. After any
  episode that moves real funds, re-sync the touched row to on-chain truth
  before finishing — a correct query against a stale row is still a wrong answer.

## Verification
Every query in a piece of work either targets a table queried earlier in the
same episode, or is preceded by its PRAGMA. If a run's failures include a
`no such column`, the PRAGMA step was skipped — that is the bug, not the schema.

## OUR INSTANCE
Record here: the on-disk paths of both databases, the current table list of
the workspace database, and any column confusion that has actually bitten this
company (with the PRAGMA-verified schema beside it).
