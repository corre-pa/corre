//! Cucumber test runner for the corre-gym e2e suite.
//!
//! Each scenario constructs a fresh `GymWorld`, which spawns a real corre-gym HTTP
//! server backed by an in-memory DB and a real LLM provider, then drives `POST /api/chat`
//! and asserts on the resulting database state.

use std::path::Path;
use std::sync::OnceLock;

use cucumber::World as _;
use e2e::corre_gym::GymWorld;

// Step definitions register themselves at compile time via cucumber's inventory-based
// macros. Bringing the modules into scope is enough; we don't need to call anything.
#[allow(unused_imports)]
use e2e::corre_gym::steps;

static LOGGER: OnceLock<()> = OnceLock::new();

fn init_logging() {
    LOGGER.get_or_init(|| {
        // Load test .env so RUST_LOG and CORRE_TEST_LLM_* are available.
        let env_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join(".env");
        let _ = dotenvy::from_filename(&env_path);

        // The corre-gym app uses `tracing`. We install a tracing_subscriber so its events
        // (LLM requests, action parsing, DB writes) are visible alongside our own logs.
        let filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,corre_gym=debug,e2e=debug"));
        let _ = tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).with_target(true).try_init();

        // env_logger is also initialised so any plain `log::*` callers (workspace deps)
        // route to the same destination. `tracing_subscriber::fmt` does not bridge the
        // `log` facade by default, so this catches anything that uses `log` directly.
        let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,corre_gym=debug,e2e=debug"))
            .is_test(false)
            .try_init();
    });
}

#[tokio::main]
async fn main() {
    init_logging();
    GymWorld::cucumber().max_concurrent_scenarios(1).fail_on_skipped().run_and_exit("tests/features/corre-gym").await;
}
