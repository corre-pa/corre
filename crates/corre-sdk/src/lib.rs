//! `corre-sdk` — the library for writing Corre capability plugins.
//!
//! Provides the CCPP v1 protocol types, the [`client::CapabilityClient`] async helper,
//! shared output types, LLM request/response structs, and utility functions for search
//! result parsing and HTML sanitization. Link this crate in your plugin binary and use
//! [`client::CapabilityClient::from_stdio`] as the entry point.

pub mod client;
pub mod codec;
pub mod html;
pub mod llm;
pub mod manifest;
pub mod protocol;
pub mod tools;
pub mod types;

// Re-export key types at crate root for convenience.
pub use client::CapabilityClient;
pub use llm::{LlmMessage, LlmRequest, LlmResponse, LlmRole};
pub use manifest::{ExecutionMode, OutputDeclaration, OutputType, PluginLink, PluginManifest, SandboxPermissions, ServiceDeclaration};
pub use protocol::ErrorCode;
pub use types::{Article, CapabilityManifest, CapabilityOutput, ContentType, CustomContent, Section, Source};
