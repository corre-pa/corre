//! Re-exports of safety configuration types from `corre-core`.
//!
//! `SafetyConfig` and `PolicyAction` are defined in `corre-core` so that the rest
//! of the workspace can depend on them without pulling in this crate.

pub use corre_core::config::{PolicyAction, SafetyConfig};
