"""Steps 1-2 of moving a VelaTerm role to herdr, for a batch: each role's VelaTerm row, its saved conversation, the
transcript baseline, the launcher's line against the role's VelaTerm preset, and whether anything holds the
conversation. Read-only.

    python -I precheck.py --machine cx1 <slug>[,<slug>...] [--out baseline.json]

The row comes from the Zulip registry's `velaterm_id` for `<machine>/<slug>`, not from the name: a machine can hold
stale rows of the same name. A Private row gets metadata only: its transcript is never opened, listed or measured.
A Codex row reports its thread and rollout instead; Codex roles have no launcher line.
"""
import argparse
import glob
import json
import os
import sqlite3
import subprocess
import sys
from pathlib import Path

HOME = Path.home()
VAULT = Path(os.environ.get('FG_VAULT_ROOT') or HOME / '_fg3')
LAUNCH = VAULT / 'Code' / 'Fg-CC-Harness' / 'plugins' / 'f-co' / 'scripts' / 'launch.mjs'
REGISTRY = HOME / '.zulip-mesh' / 'sessions.json'
PRESETS = HOME / '.config' / 'velaterm' / 'presets.json'
APP_ID = 'io.vlinx.vlxterm.fg.release'


def velaterm_db():
    """VelaTerm's database: Tauri's app data folder for the fg build, or VLX_DB."""
    if os.environ.get('VLX_DB'):
        return Path(os.environ['VLX_DB'])
    if sys.platform == 'win32':
        return Path(os.environ['APPDATA']) / APP_ID / 'vlx-term.db'
    if sys.platform == 'darwin':
        return HOME / 'Library' / 'Application Support' / APP_ID / 'vlx-term.db'
    return Path(os.environ.get('XDG_DATA_HOME') or HOME / '.local' / 'share') / APP_ID / 'vlx-term.db'


DB = velaterm_db()


def holders():
    """Command lines of every running claude and codex process, picked by executable name. Matching the command line
    would also catch another program whose arguments name `claude` and the conversation, such as a checker's script."""
    if sys.platform != 'win32':
        def ps(column):
            out = subprocess.run(['ps', '-axo', f'pid=,{column}='], capture_output=True, text=True).stdout
            return [parts for parts in (line.split(None, 1) for line in out.splitlines()) if len(parts) == 2]
        agents = {pid for pid, comm in ps('comm') if os.path.basename(comm.strip()) in ('claude', 'codex')}
        return '\n'.join(args for pid, args in ps('args') if pid in agents)
    script = ("Get-CimInstance Win32_Process | Where-Object { $_.Name -in 'claude.exe','codex.exe' } "
              "| ForEach-Object CommandLine")
    return subprocess.run(['powershell', '-NoProfile', '-Command', script], capture_output=True, text=True).stdout


def presets():
    data = json.loads(PRESETS.read_text(encoding='utf-8')) if PRESETS.exists() else []
    data = data.get('presets', data) if isinstance(data, dict) else data
    return {p['id']: p for p in data}


def claude_transcript(row, conv):
    paths = glob.glob(str(HOME / '.claude' / 'projects' / '*' / f'{conv}.jsonl'))
    if len(paths) != 1:
        row['problem'] = f'{len(paths)} transcripts'
        return
    path = Path(paths[0])
    with path.open(encoding='utf-8', errors='replace') as f:
        lines = 0
        first_cwd = None
        for line in f:
            lines += 1
            if first_cwd is None and lines <= 200:
                try:
                    first_cwd = json.loads(line).get('cwd')
                except ValueError:
                    pass
    row.update(transcript=str(path), bytes=path.stat().st_size, lines=lines, transcript_first_cwd=first_cwd)


def codex_rollout(row, thread):
    paths = glob.glob(str(HOME / '.codex' / 'sessions' / '**' / f'rollout-*{thread}.jsonl'), recursive=True)
    if len(paths) != 1:
        row['problem'] = f'{len(paths)} rollouts'
        return
    path = Path(paths[0])
    row.update(rollout=str(path), bytes=path.stat().st_size)


def launcher(row, slug, conv, preset):
    printed = subprocess.run(['node', str(LAUNCH), slug, '--resume', conv, '--print'], capture_output=True,
                             text=True)
    try:
        out = json.loads(printed.stdout)
    except ValueError:
        row['problem'] = f'launch --print: {printed.stderr.strip()[:200]}'
        return
    row['launch_cwd'] = out.get('cwd')
    row['labels'] = out.get('labels')
    if not preset:
        row['launch_matches_preset'] = None
        row['launch_argv'] = out['argv']
        return
    want = ['claude', *preset['args'], '--permission-mode', preset['permissionMode'], '--settings',
            preset['settings'], '--resume', conv]
    # The launcher appends the role's autostart message after `--`; the preset carries it separately.
    got = out['argv'][:out['argv'].index('--')] if '--' in out['argv'] else out['argv']
    row['launch_matches_preset'] = got == want
    if got != want:
        row['launch_argv'] = out['argv']
        row['preset_argv'] = want


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('slugs')
    parser.add_argument('--machine', required=True)
    parser.add_argument('--out')
    # Without a registry `velaterm_id`, also match archived rows by name (a role already moved, for a dry run).
    parser.add_argument('--archived-ok', action='store_true')
    args = parser.parse_args()

    db = sqlite3.connect(f'file:{DB}?mode=ro', uri=True)
    private = {r[0] for r in db.execute('SELECT session_id FROM session_private')}
    registry = json.loads(REGISTRY.read_text(encoding='utf-8')).get('by_name', {}) if REGISTRY.exists() else {}
    by_preset = presets()
    running = holders()

    rows = []
    for slug in args.slugs.split(','):
        name = f'{args.machine}/{slug}'
        row: dict[str, object] = {}
        row['role'] = name
        rows.append(row)
        reg = registry.get(name) or {}
        row['registry_host'] = reg.get('host')
        vid = reg.get('velaterm_id')
        cols = 'id, name, kind, engine, agent_session_id, agent_preset_id, archived_at'
        if vid:
            rec = db.execute(f'SELECT {cols} FROM sessions WHERE id=?', (vid,)).fetchone()
        else:
            live = '' if args.archived_ok else ' AND archived_at IS NULL'
            found = db.execute(f'SELECT {cols} FROM sessions WHERE name=?{live}', (slug,)).fetchall()
            rec = found[0] if len(found) == 1 else None
            row['note'] = f'no velaterm_id in the registry; {len(found)} rows named {slug}'
        if not rec:
            row['problem'] = 'no VelaTerm row'
            continue
        sid, vname, kind, engine, conv, preset_id, archived = rec
        row.update(velaterm_id=sid, velaterm_name=vname, kind=kind, engine=engine, conversation=conv,
                   preset=preset_id, archived=bool(archived), private=sid in private,
                   same_name_rows=db.execute('SELECT count(*) FROM sessions WHERE name=?', (vname,)).fetchone()[0],
                   running=bool(conv) and conv in running)
        if not conv:
            row['problem'] = 'no saved conversation'
            continue
        if row['private']:
            row['transcript'] = 'not touched (Private)'
        elif kind == 'codex':
            codex_rollout(row, conv)
        else:
            claude_transcript(row, conv)
        if kind != 'codex':
            launcher(row, slug, conv, by_preset.get(preset_id))

    if args.out:
        Path(args.out).write_text(json.dumps(rows, indent=2), encoding='utf-8')
    for r in rows:
        print(f"{r['role']:32} {r.get('kind', ''):6} {r.get('engine', ''):5} conv={str(r.get('conversation'))[:8]} "
              f"lines={r.get('lines')} bytes={r.get('bytes')} launch_ok={r.get('launch_matches_preset')} "
              f"running={r.get('running')} private={r.get('private')} archived={r.get('archived')} "
              f"rows={r.get('same_name_rows')} {r.get('note', '')} {r.get('problem', '')}")


if __name__ == '__main__':
    main()
