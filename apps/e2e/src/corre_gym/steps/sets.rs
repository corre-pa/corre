//! Assertions for recorded sets.

use cucumber::{gherkin::Step, then};

use crate::corre_gym::assertions;
use crate::corre_gym::world::GymWorld;

#[then(expr = "the following set is recorded:")]
async fn following_set_recorded(world: &mut GymWorld, step: &Step) {
    let alias = world.current_user.clone().expect("no current user");
    let user_id = world.user(&alias).expect("user lookup").user.id;
    let table = step.table.as_ref().expect("`the following set is recorded:` requires a data table");
    assertions::assert_last_set_matches(world.db(), user_id, table)
        .await
        .expect("set assertion failed");
}

/// Matches `the session has 1 set recorded`, `the session has 5 sets recorded`, with or
/// without a trailing period. Also matches the `exactly N sets are recorded` phrasing.
#[then(regex = r"^(?:the session has|exactly) (\d+) sets? (?:are )?recorded\.?$")]
async fn session_set_count(world: &mut GymWorld, expected: usize) {
    let alias = world.current_user.clone().expect("no current user");
    let user_id = world.user(&alias).expect("user lookup").user.id;
    let actual = assertions::active_session_set_count(world.db(), user_id)
        .await
        .expect("counting sets in active session");
    assert_eq!(
        actual, expected,
        "expected {expected} set(s) in active session, found {actual}. Last reply: {:?}",
        world.last_reply.as_ref().map(|r| r.text.as_str())
    );
}

/// Multi-row table assertion: exact set count + per-row field match in chronological
/// order. Row 0 is the header (column names like `exercise_type`, `count`, `value`, …).
/// Recognised column names are documented on `assertions::match_set_field`.
#[then(regex = r"^the recorded sets are:?$")]
async fn recorded_sets_are(world: &mut GymWorld, step: &Step) {
    let alias = world.current_user.clone().expect("no current user");
    let user_id = world.user(&alias).expect("user lookup").user.id;
    let table = step.table.as_ref().expect("`the recorded sets are:` requires a data table");
    assertions::assert_active_session_sets_match(world.db(), user_id, table)
        .await
        .expect("multi-row sets assertion failed");
}
