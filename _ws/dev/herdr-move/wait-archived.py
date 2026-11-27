"""Wait until a VelaTerm session is archived (a read-only poll of VelaTerm's database), then print its
`archived_at` and exit. Run it in the background after fg's Kill + Archive is due.

    python -I wait-archived.py <velaterm id>
"""
import os
import sqlite3
import sys
import time
from pathlib import Path

APP_ID = 'io.vlinx.vlxterm.fg.release'
if os.environ.get('VLX_DB'):
    DB = Path(os.environ['VLX_DB'])
elif sys.platform == 'win32':
    DB = Path(os.environ['APPDATA']) / APP_ID / 'vlx-term.db'
elif sys.platform == 'darwin':
    DB = Path.home() / 'Library' / 'Application Support' / APP_ID / 'vlx-term.db'
else:
    DB = Path(os.environ.get('XDG_DATA_HOME') or Path.home() / '.local' / 'share') / APP_ID / 'vlx-term.db'
sid = sys.argv[1]
while True:
    try:
        conn = sqlite3.connect(f'file:{DB}?mode=ro', uri=True, timeout=5)
        row = conn.execute('SELECT name, archived_at FROM sessions WHERE id=?', (sid,)).fetchone()
        conn.close()
    except sqlite3.Error as error:
        row = None
        print(f'poll error: {error}', flush=True)
    if row and row[1]:
        print(f'{row[0]} archived_at={row[1]}', flush=True)
        break
    time.sleep(3)
