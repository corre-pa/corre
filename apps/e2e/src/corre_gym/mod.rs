//! End-to-end test harness for the corre-gym app.
//!
//! Each cucumber scenario constructs a fresh [`world::GymWorld`], which spawns the real
//! corre-gym HTTP server backed by an in-memory SQLite DB and a real LLM provider. Step
//! definitions in [`steps`] drive the server through `POST /api/chat` and assert on the
//! resulting database state via the shared [`server::TestServer`] handle.

pub mod assertions;
pub mod auth;
pub mod fixtures;
pub mod http;
pub mod server;
pub mod steps;
pub mod world;

pub use world::{ChatReply, GymWorld, RegisteredUser};
