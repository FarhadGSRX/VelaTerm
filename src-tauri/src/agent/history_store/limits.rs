//! Backend-owned budgets for derived history and protocol frames.

pub const INPUT_BYTES: usize = 256 * 1024;
pub const PREVIEW_BYTES: usize = 8 * 1024;
pub const RECORD_PREVIEW_BYTES: usize = 256 * 1024;
pub const MAX_FIELDS: usize = 4096;
pub const MAX_NODES: usize = 16384;
pub const MAX_DEPTH: usize = 64;
pub const MAX_KEY_BYTES: usize = 1024;
pub const CHECKPOINT_BYTES: usize = 32 * 1024;
pub const PAGE_BYTES: usize = 512 * 1024;
pub const DETAIL_BYTES: usize = 64 * 1024;
pub const PAGE_ROWS: usize = 60;
pub const TAIL_BYTES: u64 = 8 * 1024 * 1024;
pub const SCHEMA_VERSION: u32 = 6;
pub const PARSER_VERSION: u32 = 15;
