#!/usr/bin/env python3
"""Throughput audit: where the CEO's episodes, the team's dispatches and the fuel
went over a window. Read-only. Run on the box:

    python3 scripts/throughput_audit.py [hours=24]

The metrics and targets it feeds are in docs/kernel-throughput-2026-09-02.md.
"""
import collections, json, re, sqlite3, sys, time

HOURS = float(sys.argv[1]) if len(sys.argv) > 1 else 24.0
KHAN = "/data/khan.db"
WORK = "/data/workspace/workspace.db"
since = time.strftime("%Y-%m-%dT%H:%M", time.gmtime(time.time() - HOURS * 3600))

k = sqlite3.connect(f"file:{KHAN}?mode=ro", uri=True)
try:
    w = sqlite3.connect(f"file:{WORK}?mode=ro", uri=True)
except Exception:
    w = None


def q(c, sql, args=()):
    try:
        return list(c.execute(sql, args))
    except Exception as e:
        return [("ERR", str(e))]


CHECK = ("verify", "re-verify", "recheck", "re-check", "spot-check", "spot check", "re-run", "rerun",
         "reconcil", "recon ", "audit", "sweep", "checkpoint", "confirm", "validate", "liveness", "triage")
BUILD = ("build", "ship", "write", "create", "implement", "launch", "deploy", "publish", "design",
         "redesign", "draft", "generate", "execute", "fund", "send", "submit", "post ", "fix", "migrate", "wire")


def classify(task):
    head = task[:160].lower()
    b = min((head.find(x) for x in BUILD if x in head), default=None)
    c = min((head.find(x) for x in CHECK if x in head), default=None)
    if b is not None and c is not None:
        return "build" if b <= c else "check"
    return "build" if b is not None else "check" if c is not None else "other"


print(f"window: last {HOURS:g}h since {since}Z")

# episodes
print("\n== episodes ==")
for r in q(k, "select event_kind, count(*), sum(steps), avg(steps) from episodes where started_at>? group by event_kind", (since,)):
    print("  ", r)
print("   cut-off:", q(k, "select count(*) from episodes where started_at>? and note like '%cut-off%'", (since,))[0][0])

# heartbeats that dispatched nothing
eps = q(k, "select started_at, ended_at from episodes where started_at>? and event_kind='heartbeat'", (since,))
idle = 0
for st, en in eps:
    n = q(k, "select count(*) from run_log where agent='CEO' and event in ('dispatch','delegate','delegate_parallel','hire') and ts>=? and ts<=?", (st, en or "9999"))[0][0]
    idle += n == 0
print(f"   heartbeats dispatching nothing: {idle} of {len(eps)}")
print("   heartbeat backoffs:", q(k, "select count(*) from run_log where event='heartbeat-backoff' and ts>?", (since,))[0][0])

# CEO calls + hands-on
print("\n== CEO ==")
print("   model calls:", q(k, "select count(*) from run_log where agent='CEO' and event='thinking' and ts>?", (since,))[0][0])
print("   hands-on:", q(k, "select event, count(*) from run_log where agent='CEO' and ts>? and event in ('shell','sql','khan_db_query') group by event", (since,)))
print("   exec-budget refusals:", q(k, "select count(*) from run_log where event='exec-budget' and ts>?", (since,))[0][0])
notes = [d for (d,) in q(k, "select detail from run_log where event='rate_work' and ts>?", (since,))]
print(f"   ratings {len(notes)}, self-verified {sum(1 for d in notes if re.search(r'verif|spot-check|cross-check|checked', d, re.I))}")

# dispatches
print("\n== dispatches ==")
cls = collections.Counter(); untagged = 0; shapes = collections.Counter(); byobj = collections.Counter()
for (d,) in q(k, "select detail from run_log where event in ('dispatch','delegate') and ts>?", (since,)):
    try:
        j = json.loads(d)
    except Exception:
        continue
    t = j.get("task", ""); o = j.get("objective")
    cls[classify(t)] += 1
    untagged += o is None
    byobj[o] += 1
    shapes[(o, " ".join(re.sub(r"\W+", " ", t).lower().split()[:6]))] += 1
print("   by class:", dict(cls), "| untagged:", untagged)
print("   by objective:", dict(byobj))
print("   repeated shapes >=3:", [(n, o, sh) for (o, sh), n in shapes.most_common(8) if n >= 3])
print("   refusals (repeat/check):", q(k, "select count(*) from run_log where event='dispatch-refused' and ts>?", (since,))[0][0], "| recorded by class:", q(k, "select class, count(*) from dispatches where ts>? group by class", (since,)))
print("   reports:", q(k, "select event, count(*) from run_log where event in ('background-report','routed-report') and ts>? group by event", (since,)))

# shipped
if w:
    print("\n== shipped ==")
    print("   deliverables:", q(w, "select count(*) from deliverables_log where ts>?", (since.replace("T", " "),))[0][0])
    print("   launches (pnl buy/dev buy):", q(w, "select count(*) from pnl where ts>? and (note like '%launch dev buy%' or category='buy')", (since,))[0][0])
    print("   revenue_ideas by status:", q(w, "select status, count(*) from revenue_ideas group by status"))

# fuel
print("\n== fuel ==")
rows = q(k, "select detail from run_log where event='stats' and ts>? order by id", (since,))
vals = []
for (d,) in rows:
    try:
        vals.append(json.loads(d).get("fuel_usd"))
    except Exception:
        pass
vals = [v for v in vals if isinstance(v, (int, float))]
if len(vals) >= 2:
    print(f"   tank first {vals[0]:.2f} -> last {vals[-1]:.2f}")
print("   model calls by model:", q(k, "select model, count(*) from model_calls where ts>? group by model order by 2 desc", (since,)))
