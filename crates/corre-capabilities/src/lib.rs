//! Built-in capability implementations for the Corre host.
//!
//! Contains the in-process capabilities that ship with Corre and shared tool helpers.
//! The subprocess host and capability registry live in `corre-host`.
//!
//! # Modules
//!
//! | Module | Description |
//! |--------|-------------|
//! | [`daily_brief`] | Daily Research Brief — multi-step pipeline that reads `topics.yml`, searches the web via Brave Search MCP, deduplicates and LLM-scores results, summarises the top stories, and emits a newspaper edition |
//! | [`rolodex`] | Rolodex — automated personal contact engagement. Checks a SQLite database for due outreach strategies (birthdays, news, check-ins) and executes each one |
//! | [`rolodex_db`] | SQLite schema and query helpers for the Rolodex contact database |
//! | [`rolodex_import`] | CSV, vCard, and JSON import parsers for populating the contact database |
//! | [`tools`] | Re-exports shared utility functions from `corre-sdk::tools` |

pub mod daily_brief;
pub mod rolodex;
pub mod rolodex_db;
pub mod rolodex_import;
pub mod tools;
