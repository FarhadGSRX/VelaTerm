"""Does VelaTerm's search index still hold text for conversations whose transcript is gone? Read-only: prints counts
and date ranges only, never text. Open nothing else, and copy the index aside first if it matters, since VelaTerm's
next refresh of such a session can delete its rows.

    python -I index-copies.py
"""
import datetime
import glob
import os
import sqlite3
import sys
from pathlib import Path

APP_ID = 'io.vlinx.vlxterm.fg.release'
if sys.platform == 'win32':
    DATA = Path(os.environ['APPDATA']) / APP_ID
elif sys.platform == 'darwin':
    DATA = Path.home() / 'Library' / 'Application Support' / APP_ID
else:
    DATA = Path(os.environ.get('XDG_DATA_HOME') or Path.home() / '.local' / 'share') / APP_ID


def day(ts):
    if ts is None:
        return '-'
    ts = int(ts)
    return datetime.datetime.fromtimestamp(ts / 1000 if ts > 10**11 else ts).strftime('%Y-%m-%d')


for index in sorted(glob.glob(str(DATA / 'search-index' / 'corpus*.sqlite'))):
    db = sqlite3.connect(f'file:{index}?mode=ro', uri=True)
    states = db.execute('SELECT session_id, source, path FROM corpus_state').fetchall()
    gone = [(sid, path) for sid, source, path in states if source == 'transcript' and path and not os.path.exists(path)]
    print(f'{Path(index).name}: {len(states)} sessions indexed; {len(gone)} with a transcript path that no longer exists')
    total = 0
    for sid, _ in gone:
        rows, first, last = db.execute('SELECT count(*), min(ts), max(ts) FROM session_fts WHERE session_id=?',
                                       (sid,)).fetchone()
        total += rows
        print(f'  session {sid[:8]}: {rows} indexed messages, {day(first)} .. {day(last)}')
    print(f'  {total} indexed messages in all for the missing transcripts')
