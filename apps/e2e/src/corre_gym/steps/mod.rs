//! Cucumber step definitions, organised by concern. Steps register themselves at compile
//! time via cucumber's `inventory`-backed macros, so this module just needs to be brought
//! into scope by the test binary (`tests/e2e-corre-gym.rs`).

pub mod background;
pub mod chat;
pub mod entries;
pub mod goals;
pub mod health;
pub mod rest_timer;
pub mod session;
pub mod sets;
