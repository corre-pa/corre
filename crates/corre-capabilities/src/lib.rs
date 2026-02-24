//! Capability implementations and the `CapabilityRegistry`.
//!
//! Contains the Rolodex capability, a registry that maps capability names to trait
//! objects, and shared helpers re-exported from `corre-sdk`.

pub mod daily_brief;
pub mod registry;
pub mod rolodex;
pub mod rolodex_db;
pub mod rolodex_import;
pub mod subprocess;
pub mod tools;
