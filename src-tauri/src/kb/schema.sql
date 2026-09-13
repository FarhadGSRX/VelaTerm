CREATE TABLE IF NOT EXISTS kb_vaults (
 id TEXT PRIMARY KEY, name TEXT NOT NULL, root TEXT NOT NULL UNIQUE, created_at INTEGER NOT NULL
);
-- File contents are a disposable search cache. Reads and saves always check the filesystem.
CREATE TABLE IF NOT EXISTS kb_files (
 vault_id TEXT NOT NULL REFERENCES kb_vaults(id) ON DELETE CASCADE,
 path TEXT NOT NULL, content TEXT NOT NULL, digest TEXT NOT NULL,
 mtime TEXT NOT NULL, size INTEGER NOT NULL, PRIMARY KEY(vault_id,path)
);
CREATE TABLE IF NOT EXISTS kb_favorites (
 vault_id TEXT NOT NULL REFERENCES kb_vaults(id) ON DELETE CASCADE,
 path TEXT NOT NULL, PRIMARY KEY(vault_id,path)
);
CREATE TABLE IF NOT EXISTS kb_trash (
 id TEXT PRIMARY KEY, vault_id TEXT NOT NULL REFERENCES kb_vaults(id) ON DELETE CASCADE,
 path TEXT NOT NULL, deleted_at INTEGER NOT NULL
);

-- Closing a notebook hides its registration without losing favorites or trash provenance.
CREATE TABLE IF NOT EXISTS kb_closed_vaults (
 vault_id TEXT PRIMARY KEY REFERENCES kb_vaults(id) ON DELETE CASCADE
);

-- Import history. Header rows keep the batch outcome; file rows keep the per-file result.
-- These rows are audit data only: the filesystem remains the source of truth for notes.
CREATE TABLE IF NOT EXISTS kb_imports (
 id TEXT PRIMARY KEY,
 vault_id TEXT NOT NULL REFERENCES kb_vaults(id) ON DELETE CASCADE,
 destination TEXT NOT NULL, status TEXT NOT NULL, error TEXT NOT NULL DEFAULT '',
 files INTEGER NOT NULL DEFAULT 0, imported INTEGER NOT NULL DEFAULT 0,
 skipped INTEGER NOT NULL DEFAULT 0, bytes INTEGER NOT NULL DEFAULT 0,
 created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, finished_at INTEGER
);
CREATE INDEX IF NOT EXISTS kb_imports_vault ON kb_imports(vault_id, created_at);
CREATE TABLE IF NOT EXISTS kb_import_files (
 import_id TEXT NOT NULL REFERENCES kb_imports(id) ON DELETE CASCADE,
 path TEXT NOT NULL, size INTEGER NOT NULL, status TEXT NOT NULL, reason TEXT NOT NULL DEFAULT '',
 PRIMARY KEY(import_id, path)
);
