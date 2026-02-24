//! `corre-core` -- shared traits, types, and configuration for the Corre workspace.
//!
//! This crate sits at the bottom of the dependency graph with no internal Corre dependencies.
//! Every other crate imports its abstractions (`Capability`, `McpCaller`, `LlmProvider`) and
//! publishing types (`Edition`, `Section`, `Article`) rather than depending on one another.

pub mod capability;
pub mod config;
pub mod plugin;
pub mod sandbox;
pub mod scheduler;
pub mod secret;
pub mod service;
pub mod tracker;
