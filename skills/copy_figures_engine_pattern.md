Load before editing, registering, or debugging any registered tool that wraps a script on disk: the registry re-materializes a tool's file on every call, so a tool that writes to its own path copies itself over its engine and recurses forever.

# tool-wraps-a-script — the engine-separate registration pattern

A registered tool is not a file you own. The registry persists the tool's
script to its own path and REWRITES it from the registry on every call.
Anything that assumes the file on disk is still what you put there is
building on sand.

## When to use
Before editing, re-registering, or debugging any tool whose script is a thin
wrapper around a larger program on disk — audits, sweeps, generators, any
job too big to live inside the registered script.

## The law, paid for twice in one hour
A sweep tool's wrapper copied its own registered path onto the engine path at
call time. By then the registry had already replaced that path with the
wrapper itself, so the wrapper copied itself over the engine and then ran
itself: infinite recursion, one crash log, no output. The same root cause
nearly landed again the same hour when a hand-written wrapper was placed on
top of a working disk engine.

**Never point a tool's execution at its own registered path.** Not to copy
from it, not to read it, not to run it.

## The engine-separate pattern (the only safe shape)
- `<name>_engine.py` — the real logic, a plain file on disk, **never
  registered as a tool** and never written by the wrapper.
- `<name>.py` — the registered wrapper. It reads the tool's argument
  environment variable, builds `sys.argv`, and hands off with
  `runpy.run_path(".../<name>_engine.py", run_name="__main__")`, catching
  `SystemExit` for the exit code. Nothing else.
- To ship new logic, **edit the engine file directly** and smoke-test it as a
  plain script (`python3 <name>_engine.py <args>`) BEFORE any wrapper call.
  The wrapper never changes.

## Diagnostic tell
A registered tool that errors with `RecursionError`, or whose disk copy has
shrunk to wrapper size and contains `runpy` where the engine used to be, has
the self-copy bug back. Read the crash log, restore the engine from a backup,
and re-apply the pattern — do not patch the wrapper in place.

## Pitfalls
- Keep the canon rules the tool enforces IN THE ENGINE. A rule that lives in
  the wrapper is deleted by the next re-registration.
- Take a backup of the engine before any edit, named for the reason. The
  registry backs up nothing.
- The same trap applies to any tool that writes into its own directory —
  logs, caches, staged output. Write those somewhere the registry does not
  own.

## Verification
Run the engine directly and confirm the expected exit code, then call the
registered tool and confirm the same result. A wrapper whose disk size is
roughly that of the engine means the two have been confused again.

## OUR INSTANCE
Record here: the engine and wrapper paths for each tool in this family, where
their backups live, and the crash-log locations of any past recursion
incident.
