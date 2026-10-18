//! Shared bounded reads for native history and live protocol records.

pub mod limits;
pub mod projection;
pub mod source;
pub mod store;
pub mod adapter;
pub mod api;
pub mod live_view;
pub mod search;
pub mod live;
pub mod protocol_source;
pub mod sqlite_source;
mod native_database;
mod kiro_database;
mod kiro_pair;
pub mod reconciliation;
pub mod attachments;
pub mod children;
pub mod rewind;
mod shell_text;
pub mod compat;
pub mod event_stream;
pub mod markdown;
pub mod exports;
pub mod prose;
pub mod prose_reader;
pub mod context;
pub mod refer;
mod ask;
mod scheduler;
mod readers;
mod recovery;
mod cache;
pub mod work;
mod antigravity_adapter;

#[cfg(test)]
mod tests;
