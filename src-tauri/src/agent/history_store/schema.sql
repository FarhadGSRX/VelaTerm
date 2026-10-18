PRAGMA journal_mode=WAL;
PRAGMA synchronous=NORMAL;
PRAGMA cache_size=-8192;
PRAGMA wal_autocheckpoint=512;
CREATE TABLE IF NOT EXISTS index_meta (version INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS generation_access(source_id TEXT NOT NULL,generation TEXT NOT NULL,touched INTEGER NOT NULL,
    PRIMARY KEY(source_id,generation));
INSERT INTO index_meta SELECT 6 WHERE NOT EXISTS (SELECT 1 FROM index_meta);
CREATE TABLE IF NOT EXISTS sources (
    id TEXT PRIMARY KEY, provider TEXT NOT NULL, native_id TEXT NOT NULL,
    path TEXT NOT NULL UNIQUE, identity TEXT NOT NULL, generation TEXT NOT NULL,
    size INTEGER NOT NULL, modified TEXT NOT NULL, head_hash BLOB NOT NULL,
    indexed_bytes INTEGER NOT NULL DEFAULT 0, status TEXT NOT NULL DEFAULT 'partial',
    error TEXT, parser_version INTEGER NOT NULL, branch_revision TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS records (
    source_id TEXT NOT NULL, generation TEXT NOT NULL, start INTEGER NOT NULL,
    end INTEGER NOT NULL, ordinal INTEGER, native_id TEXT, parent_id TEXT, event_type TEXT,
    projection TEXT, error TEXT, active INTEGER NOT NULL DEFAULT 1, PRIMARY KEY(source_id,generation,start)
);
CREATE INDEX IF NOT EXISTS record_ordinals ON records(source_id,generation,ordinal);
CREATE INDEX IF NOT EXISTS record_identity ON records(source_id,generation,native_id);
CREATE INDEX IF NOT EXISTS record_parents ON records(source_id,generation,parent_id);
CREATE TABLE IF NOT EXISTS fields (
    ref TEXT PRIMARY KEY, source_id TEXT NOT NULL, generation TEXT NOT NULL,
    record_start INTEGER NOT NULL, path TEXT NOT NULL, raw_start INTEGER NOT NULL,
    raw_end INTEGER NOT NULL, decoded_bytes INTEGER NOT NULL, parent_ref TEXT, encoding TEXT NOT NULL DEFAULT 'string',
    hash BLOB, UNIQUE(source_id,generation,record_start,path)
);
CREATE TABLE IF NOT EXISTS checkpoints (
    source_id TEXT NOT NULL, generation TEXT NOT NULL, record_start INTEGER NOT NULL,
    path TEXT NOT NULL, raw_start INTEGER NOT NULL, raw_end INTEGER NOT NULL,
    decoded_start INTEGER NOT NULL, decoded_end INTEGER NOT NULL, hash BLOB NOT NULL, sequence INTEGER NOT NULL,
    PRIMARY KEY(source_id,generation,record_start,path,decoded_start)
);
CREATE INDEX IF NOT EXISTS checkpoint_sequences ON checkpoints(source_id,generation,record_start,path,sequence);
CREATE TABLE IF NOT EXISTS source_blocks (
    source_id TEXT NOT NULL, generation TEXT NOT NULL, start INTEGER NOT NULL,
    end INTEGER NOT NULL, hash BLOB NOT NULL, PRIMARY KEY(source_id,generation,start)
);
CREATE TABLE IF NOT EXISTS branches (
    session_scope TEXT PRIMARY KEY, revision TEXT NOT NULL, complete INTEGER NOT NULL DEFAULT 0,
    error TEXT
);
CREATE TABLE IF NOT EXISTS segments (
    session_scope TEXT NOT NULL, sequence INTEGER NOT NULL, source_id TEXT NOT NULL,
    generation TEXT NOT NULL, end_byte INTEGER, end_ordinal INTEGER,
    PRIMARY KEY(session_scope,sequence)
);
CREATE TABLE IF NOT EXISTS rows (
    source_id TEXT NOT NULL, generation TEXT NOT NULL, id TEXT NOT NULL,
    order_key TEXT NOT NULL, record_start INTEGER NOT NULL, record_end INTEGER NOT NULL,
    ordinal INTEGER, kind TEXT NOT NULL, call_id TEXT, revision INTEGER NOT NULL DEFAULT 1,
    summary TEXT NOT NULL, PRIMARY KEY(source_id,generation,id)
);
CREATE INDEX IF NOT EXISTS ordered_rows ON rows(source_id,generation,order_key);
CREATE INDEX IF NOT EXISTS scoped_calls ON rows(source_id,generation,call_id);
CREATE TABLE IF NOT EXISTS row_parts (
    source_id TEXT NOT NULL, generation TEXT NOT NULL, row_id TEXT NOT NULL,
    record_start INTEGER NOT NULL, record_end INTEGER NOT NULL, ordinal INTEGER,
    summary TEXT NOT NULL, PRIMARY KEY(source_id,generation,row_id,record_start)
);
-- Prose locates the row a record produced.
CREATE INDEX IF NOT EXISTS record_parts ON row_parts(source_id,generation,record_start);
CREATE TABLE IF NOT EXISTS row_fields (
    source_id TEXT NOT NULL, generation TEXT NOT NULL, row_id TEXT NOT NULL,
    ref TEXT NOT NULL, label TEXT NOT NULL, sequence INTEGER NOT NULL,
    PRIMARY KEY(source_id,generation,row_id,ref)
);
CREATE INDEX IF NOT EXISTS row_details ON row_fields(source_id,generation,row_id,sequence);
-- A detail read checks that its reference belongs to a row.
CREATE INDEX IF NOT EXISTS field_rows ON row_fields(source_id,generation,ref,row_id);
CREATE TABLE IF NOT EXISTS files (
    source_id TEXT NOT NULL, generation TEXT NOT NULL, row_id TEXT NOT NULL,
    file_ordinal INTEGER NOT NULL, path TEXT NOT NULL, change_type TEXT,
    detail_ref TEXT, detail_kind TEXT, record_start INTEGER NOT NULL,
    PRIMARY KEY(source_id,generation,row_id,record_start,file_ordinal)
);
CREATE TABLE IF NOT EXISTS spool_entries (
    ref TEXT PRIMARY KEY, session_scope TEXT NOT NULL, path TEXT NOT NULL,
    bytes INTEGER NOT NULL, reconciled INTEGER NOT NULL DEFAULT 0, reason TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS protocol_sources (
    source_id TEXT PRIMARY KEY, session_scope TEXT NOT NULL, process_epoch INTEGER NOT NULL,
    reference TEXT NOT NULL UNIQUE
);
CREATE INDEX IF NOT EXISTS protocol_scopes ON protocol_sources(session_scope,process_epoch);
CREATE TABLE IF NOT EXISTS protocol_aliases (
    source_id TEXT NOT NULL, generation TEXT NOT NULL, view_id TEXT NOT NULL,
    row_id TEXT NOT NULL, PRIMARY KEY(source_id,generation,view_id)
);
-- A row another row stands for in the conversation view. It stays addressable for detail and prose
-- reads; pages leave it out, and locating it lands on its replacement.
CREATE TABLE IF NOT EXISTS superseded_rows (
    source_id TEXT NOT NULL, generation TEXT NOT NULL, row_id TEXT NOT NULL,
    replacement TEXT NOT NULL, PRIMARY KEY(source_id,generation,row_id)
);
-- How far the active branch of a tree recording was selected over a complete scan. While an appended
-- tail is still being read, pages show the branch up to here instead of nothing.
CREATE TABLE IF NOT EXISTS branch_marks (
    source_id TEXT NOT NULL, generation TEXT NOT NULL, selected_end INTEGER NOT NULL,
    PRIMARY KEY(source_id,generation)
);
CREATE TABLE IF NOT EXISTS protocol_state (
    source_id TEXT NOT NULL, generation TEXT NOT NULL, key TEXT NOT NULL,
    value TEXT NOT NULL, PRIMARY KEY(source_id,generation,key)
);
CREATE TABLE IF NOT EXISTS append_fields (ref TEXT PRIMARY KEY);

CREATE TABLE IF NOT EXISTS cursor_keys (id INTEGER PRIMARY KEY CHECK(id=1), secret TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS native_pending (
    source_id TEXT NOT NULL, generation TEXT NOT NULL, record_start INTEGER NOT NULL,
    sequence INTEGER NOT NULL, row_id TEXT NOT NULL, active INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY(source_id,generation,record_start,sequence)
);
CREATE INDEX IF NOT EXISTS pending_order ON native_pending(source_id,generation,active,record_start,sequence);

CREATE TABLE IF NOT EXISTS native_db_sources (
    source_id TEXT PRIMARY KEY,path TEXT NOT NULL,native_id TEXT NOT NULL,entry_count INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS native_db_entries (
    source_id TEXT NOT NULL,generation TEXT NOT NULL,record_start INTEGER NOT NULL,record_end INTEGER NOT NULL,
    entry_kind INTEGER NOT NULL,native_rowid INTEGER NOT NULL,entry_id TEXT NOT NULL,message_id TEXT NOT NULL,byte_count INTEGER NOT NULL,
    PRIMARY KEY(source_id,generation,record_start)
);
CREATE INDEX IF NOT EXISTS native_db_message_entries ON native_db_entries(source_id,generation,message_id,entry_kind);
