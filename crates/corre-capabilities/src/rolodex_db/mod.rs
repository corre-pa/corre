//! SQLite persistence layer for the Corre contact and outreach system.
//!
//! Provides the `Database` handle, domain model types, and all CRUD operations for
//! contacts, outreach strategies, outreach logs, and contact profile entries.

pub mod contacts;
pub mod db;
pub mod models;
pub mod profiles;
pub mod strategies;

pub use db::Database;
pub use models::{
    Contact, ContactMethod, Importance, OutreachLog, OutreachStrategy, ProfileCategory, ProfileEntry, ProfileSource, StrategyType,
};
