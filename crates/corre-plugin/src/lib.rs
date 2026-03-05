//! Built-in app implementations for the Corre host.
//!
//! Contains the in-process apps that ship with Corre and shared tool helpers.
//! The subprocess host and app registry live in `corre-host`.
//!
//! # Modules
//!
//! | Module | Description |
//! |--------|-------------|
//! | [`daily_brief`] | Daily Research Brief — multi-step pipeline that reads `topics.yml`, searches the web via Brave Search MCP, deduplicates and LLM-scores results, summarises the top stories, and emits a newspaper edition |
//! | [`subprocess`] | Subprocess-based app host for plugins speaking CCPP v1/v2 |
//! | [`registry`] | App registry mapping names to trait objects |
//! | [`tools`] | Re-exports shared utility functions from `corre-sdk::tools` |

pub mod daily_brief;
pub mod registry;
pub mod subprocess;
pub mod tools;
