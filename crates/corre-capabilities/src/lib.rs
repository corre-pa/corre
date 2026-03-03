//! Capability implementations.
//!
//! Contains built-in capabilities (Rolodex, daily-brief) and shared helpers.
//! The subprocess host and capability registry have moved to `corre-host`.

pub mod daily_brief;
pub mod rolodex;
pub mod rolodex_db;
pub mod rolodex_import;
pub mod tools;
