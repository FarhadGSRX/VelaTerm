#!/usr/bin/env python3
"""relocate-roots.py: rewrite VelaTerm's saved paths after a folder root moves, e.g. C:\\_fg3 -> C:\\Users\\me\\_fg3.

VelaTerm stores project roots, session working directories, worktree paths and Claude launch arguments as
plain strings in vlx-term.db. A moved root leaves them all pointing at the old place. This rewrites the
columns that hold configuration and leaves conversation text (search indexes, transcripts, memory) alone.

Run it with every VelaTerm edition fully closed: a running app writes its own copy of the settings back on exit.

  python relocate-roots.py --map "C:\\_fg3=C:\\Users\\me\\_fg3" [--map OLD=NEW ...] [--data-dir DIR ...] [--apply]

Without --apply it only reports what would change. With --apply it first saves a backup beside each database
(vlx-term.db.pre-relocate-<time>), rewrites inside one transaction, then checkpoints the WAL. With no
--data-dir it handles every %APPDATA%\\io.vlinx.vlxterm* folder that holds a vlx-term.db.
"""
import argparse
import datetime
import glob
import json
import os
import re
import sqlite3
import sys

# Columns that hold paths as configuration. app_settings.value is JSON and is rewritten string by string.
TARGETS = [
    ("projects", "root_path"),
    ("sessions", "cwd"),
    ("sessions", "worktree_path"),
    ("sessions", "agent_args"),
    ("groups", "worktree_path"),
    ("spawn_worktrees", "*"),
    ("kb_vaults", "*"),
    ("agent_presets", "*"),
    ("app_settings", "value"),
]
# Conversation text that merely mentions a path. Rewriting it would falsify history.
SKIP = re.compile(r"fts|^session_words|messages|^memory_|^chat_submissions|^plan_execute_|^session_tell", re.I)


def build_rules(maps):
    """One (regex, replacement-by-separator) per mapping, matching \\, / and Git Bash /c/ spellings."""
    rules = []
    for m in maps:
        if "=" not in m:
            sys.exit(f"--map needs OLD=NEW: {m}")
        old, new = (p.rstrip("\\/") for p in m.split("=", 1))
        o = re.match(r"^([A-Za-z]):[\\/](.+)$", old)
        n = re.match(r"^([A-Za-z]):[\\/](.+)$", new)
        if not o or not n:
            sys.exit(f"--map paths must be absolute drive paths: {m}")
        old_parts = re.split(r"[\\/]", o.group(2))
        new_parts = re.split(r"[\\/]", n.group(2))
        sep = r"(?:\\\\|\\|/)"
        tail = r"(?=[\\/\"'\s;,]|$)"
        body = sep.join(re.escape(p) for p in old_parts)
        win = re.compile(rf"(?i)\b{re.escape(o.group(1))}:({sep}){body}{tail}")
        bash = re.compile(rf"(?i)/{re.escape(o.group(1).lower())}/{'/'.join(re.escape(p) for p in old_parts)}{tail}")

        def win_sub(mt, d=n.group(1), parts=new_parts):
            s = mt.group(1)
            return f"{d}:{s}{s.join(parts)}"

        def bash_sub(mt, d=n.group(1).lower(), parts=new_parts):
            return f"/{d}/{'/'.join(parts)}"

        rules.append((win, win_sub))
        rules.append((bash, bash_sub))
    return rules


def rewrite(text, rules):
    for rx, sub in rules:
        text = rx.sub(sub, text)
    return text


def rewrite_json(value, rules):
    """Rewrite strings inside a JSON document, or None when the value is not JSON that round-trips exactly."""
    try:
        doc = json.loads(value)
    except (ValueError, TypeError):
        return None
    if json.dumps(doc, ensure_ascii=False, separators=(",", ":")) != value:
        return None

    def walk(x):
        if isinstance(x, str):
            return rewrite(x, rules)
        if isinstance(x, list):
            return [walk(v) for v in x]
        if isinstance(x, dict):
            return {rewrite(k, rules): walk(v) for k, v in x.items()}
        return x

    return json.dumps(walk(doc), ensure_ascii=False, separators=(",", ":"))


def columns(con, table):
    return [r[1] for r in con.execute(f"pragma table_info('{table}')")]


def tables(con):
    return [r[0] for r in con.execute("select name from sqlite_master where type='table'")]


def scan(con, rules):
    """Every text cell any rule matches, as {(table, column): count}, over all tables."""
    hits = {}
    for t in tables(con):
        cols = columns(con, t)
        for row in con.execute(f"select * from '{t}'"):
            for c, v in zip(cols, row):
                if isinstance(v, str) and any(rx.search(v) for rx, _ in rules):
                    hits[(t, c)] = hits.get((t, c), 0) + 1
    return hits


def plan(con, rules):
    """The (table, column, rowid, old, new) edits for the target columns."""
    edits = []
    present = set(tables(con))
    for t, col in TARGETS:
        if t not in present:
            continue
        cols = columns(con, t) if col == "*" else [col]
        for c in cols:
            for rowid, v in con.execute(f"select rowid, \"{c}\" from '{t}'"):
                if not isinstance(v, str):
                    continue
                if t == "app_settings":
                    new = rewrite_json(v, rules)
                    if new is None:
                        new = rewrite(v, rules) if not v.lstrip().startswith(("{", "[")) else v
                        if new == v and any(rx.search(v) for rx, _ in rules):
                            print(f"    ! app_settings rowid {rowid}: JSON does not round-trip; left for a manual edit")
                else:
                    new = rewrite(v, rules)
                if new != v:
                    edits.append((t, c, rowid, v, new))
    return edits


def report(title, hits):
    print(f"  {title}:")
    if not hits:
        print("    (none)")
    for (t, c), n in sorted(hits.items()):
        tag = "" if not SKIP.search(t) else "   [conversation text, left alone]"
        print(f"    {t}.{c}: {n}{tag}")


def process(db, rules, apply):
    print(f"== {db}")
    # A dry run opens the database read-only, so it is safe beside a running VelaTerm.
    uri = "file:" + db.replace("\\", "/") + ("" if apply else "?mode=ro")
    con = sqlite3.connect(uri, uri=True, timeout=5)
    before = scan(con, rules)
    report("matches before", before)
    edits = plan(con, rules)
    for t, c, rowid, old, new in edits:
        short = lambda s: s if len(s) <= 160 else s[:157] + "..."
        if t == "app_settings":
            print(f"    ~ {t}.{c} rowid {rowid} (JSON, {len(old)} -> {len(new)} chars)")
        else:
            print(f"    ~ {t}.{c} rowid {rowid}: {short(old)}  ->  {short(new)}")
    if not apply:
        print(f"  dry run: {len(edits)} cell(s) would change")
        con.close()
        return
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    backup = f"{db}.pre-relocate-{stamp}"
    dst = sqlite3.connect(backup)
    con.backup(dst)
    dst.close()
    print(f"  backup: {backup}")
    with con:
        for t, c, rowid, old, new in edits:
            con.execute(f"update '{t}' set \"{c}\" = ? where rowid = ? and \"{c}\" = ?", (new, rowid, old))
    con.execute("pragma wal_checkpoint(truncate)")
    after = scan(con, rules)
    report("matches after", after)
    left = {k: n for k, n in after.items() if not SKIP.search(k[0])}
    if left:
        print("  ! configuration matches remain (not in the target list); review them by hand")
    print(f"  applied: {len(edits)} cell(s) changed")
    con.close()


def running_velaterm():
    """Names of running VelaTerm processes. An idle app holds no database lock in WAL mode, so ask Windows."""
    if os.name != "nt":
        return []
    import subprocess
    out = subprocess.run(["tasklist", "/fo", "csv", "/nh"], capture_output=True, text=True).stdout
    return sorted({line.split('","')[0].strip('"') for line in out.splitlines() if line.lower().startswith('"velaterm')})


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--map", action="append", required=True, help="OLD=NEW root, repeatable")
    ap.add_argument("--data-dir", action="append", help="VelaTerm data dir or a vlx-term.db, repeatable")
    ap.add_argument("--apply", action="store_true", help="write the changes (default: report only)")
    a = ap.parse_args()
    rules = build_rules(a.map)
    targets = a.data_dir or glob.glob(os.path.join(os.environ.get("APPDATA", ""), "io.vlinx.vlxterm*"))
    dbs = []
    for t in targets:
        db = t if t.endswith(".db") else os.path.join(t, "vlx-term.db")
        if os.path.isfile(db):
            dbs.append(db)
        else:
            print(f"(no vlx-term.db in {t})")
    if not dbs:
        sys.exit("no VelaTerm database found")
    # A copy elsewhere (a test run) may be rewritten while VelaTerm runs; a live data dir may not.
    appdata = os.path.normcase(os.path.abspath(os.environ.get("APPDATA", "") or os.sep))
    live = [db for db in dbs if os.path.normcase(os.path.abspath(db)).startswith(appdata + os.sep)]
    running = running_velaterm()
    if a.apply and live and running:
        sys.exit(f"close VelaTerm first; still running: {', '.join(running)}")
    for db in dbs:
        process(db, rules, a.apply)


if __name__ == "__main__":
    main()
