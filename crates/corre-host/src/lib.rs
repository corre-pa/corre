//! Subprocess capability host and plugin registry.
//!
//! Contains the [`SubprocessCapability`](subprocess::SubprocessCapability) — the generic host
//! that spawns plugin binaries and brokers their CCPP requests — and the
//! [`CapabilityRegistry`](registry::CapabilityRegistry) that maps capability names to trait
//! objects.

pub mod registry;
pub mod subprocess;
