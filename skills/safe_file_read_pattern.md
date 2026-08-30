Load BEFORE ANY file read: the built-in read_file tool can ENOENT on files that exist. Use shell reads or an absolute-path fallback tool; never debug the file or blind-retry read_file.
# Safe file read pattern

## Problem
The built-in `read_file` tool can fail with `No such file or directory (os error 2)` on files that demonstrably exist (ls confirms, python open() reads fine). Observed failure rates up to ~50% in a session. This is a tool path-resolution quirk, NOT a bug in your script or a missing file — do not spend iterations debugging the file.

## Solution, in order
1. Quick peek: `cd` to the workspace root and `sed -n '1,40p' path/to/file`.
2. Full reliable read: build (or reuse) a fallback read tool that tries the path as given, then workspace-rooted, then the nested-duplicate variant, and returns size + sha256 + content.
3. Binary files (PNGs, screenshots): read_file needs UTF-8 and will fail anyway — inspect with `ls -la` + `file`, never read_file a binary.

## Writing (companion rule)
- ALWAYS use ABSOLUTE paths with write_file. Relative paths resolve against a drifted CWD and can silently write to a nested `<root>/<root>/...` duplicate path.
- After any write, verify with `ls -la` + `sha256sum` at the absolute path.
- Before any publish/send, sweep for nested duplicates: `find <root>/<first-dir> -type f`.

## Gotcha
If the fallback read also fails while `ls` shows the file, the path you were handed is wrong — look for the nested-duplicate copy. Do NOT assume the file is lost.
