//! `corre-news` -- the CorreNews web server and edition storage layer.
//!
//! Persists capability output as dated JSON editions, indexes articles for full-text search,
//! and serves a newspaper-style HTML interface over HTTP via Axum.

pub mod archive;
pub mod cache;
pub mod config;
pub mod edition;
pub mod render;
pub mod search;
pub mod server;

pub use config::NewsConfig;
pub use edition::Edition;
