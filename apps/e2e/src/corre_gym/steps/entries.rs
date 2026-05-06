//! Step definitions that inspect or mutate `exercise_entry` and `sessions` rows
//! beyond the per-set assertions in `steps/sets.rs`. Covers:
//!
//!  * open-entry counts in the active session
//!  * per-exercise entry status (open / closed / none)
//!  * back-dating timestamps to simulate a long break (12-hour cutoff scenarios)
//!  * a soft text match for "is this a new session?" assistant replies

use cucumber::{given, then};

use crate::corre_gym::assertions::{
    EntryState, entry_state_for_exercise, open_entry_count, reply_asks_about_new_session, rewind_user_activity,
};
use crate::corre_gym::world::GymWorld;

/// Matches `there is 1 open entry`, `there are 0 open entries`, optionally
/// suffixed with `in the active session` for clarity in the feature file.
#[then(regex = r"^there (?:is|are) (\d+) open entr(?:y|ies)(?: in the active session)?$")]
async fn open_entry_count_step(world: &mut GymWorld, expected: usize) {
    let alias = world.current_user.clone().expect("no current user — run a When step first");
    let user_id = world.user(&alias).expect("user lookup").user.id;
    let actual = open_entry_count(world.db(), user_id).await.expect("counting open entries");
    assert_eq!(
        actual,
        expected,
        "expected {expected} open entr{} in the active session, found {actual}. Last reply: {:?}",
        if expected == 1 { "y" } else { "ies" },
        world.last_reply.as_ref().map(|r| r.text.as_str()),
    );
}

/// `Then the entry for "Bench Press" is open` / `is closed`. Resolves the
/// exercise name through the same hierarchy rules used by the per-set matcher
/// (direct match plus descendants when the resolved type is not a Variation).
#[then(regex = r#"^the entry for "(.+)" is (open|closed)$"#)]
async fn entry_for_exercise_status(world: &mut GymWorld, exercise: String, status: String) {
    let alias = world.current_user.clone().expect("no current user — run a When step first");
    let user_id = world.user(&alias).expect("user lookup").user.id;
    let actual = entry_state_for_exercise(world.db(), user_id, &exercise).await.expect("looking up entry state");
    let expected = match status.as_str() {
        "open" => EntryState::Open,
        "closed" => EntryState::Closed,
        other => panic!("unknown entry status `{other}` (expected `open` or `closed`)"),
    };
    assert_eq!(
        actual,
        expected,
        "expected entry for `{exercise}` to be {status:?}, found {actual:?}. Last reply: {:?}",
        world.last_reply.as_ref().map(|r| r.text.as_str()),
    );
}

/// `Given 24 hours have passed since the last activity` — back-dates every
/// session/entry/set timestamp for the current user by N hours. Used by the
/// 12-hour SESSION CONTINUITY scenarios to simulate a real-world break without
/// pausing the test runner. Tolerates the singular `1 hour` and plural form.
#[given(regex = r"^(\d+) hours? have passed since the last activity$")]
async fn rewind_activity_step(world: &mut GymWorld, hours: i64) {
    let alias = world.current_user.clone().expect("no current user — run a When step first");
    let user_id = world.user(&alias).expect("user lookup").user.id;
    rewind_user_activity(world.db(), user_id, hours).await.expect("rewinding user activity");
}

/// `Then the assistant asks whether to start a new session`. Soft text match
/// against the last assistant reply — the regex set in
/// `reply_asks_about_new_session` is the source of truth for what counts as
/// "asking".
#[then(regex = r"^the assistant asks whether to start a new session$")]
async fn assistant_asks_new_session(world: &mut GymWorld) {
    let reply = world.last_reply.as_ref().expect("no assistant reply yet — run a When step first");
    assert!(
        reply_asks_about_new_session(&reply.text),
        "expected the assistant's reply to ask about a new session, but got: {:?}",
        reply.text,
    );
}
