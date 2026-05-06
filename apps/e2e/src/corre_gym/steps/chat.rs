//! Chat steps: drive `POST /api/chat` with canned phrasings and free-form messages.

use cucumber::when;

use crate::corre_gym::http;
use crate::corre_gym::world::GymWorld;

/// Free-form message — exact wording from `log_workout.feature`. The verb is `send` (not
/// `sends`) to match the example feature; we add a `sends` alternative below for new
/// features.
#[when(regex = r#"^(\w+) send(?:s)? a telegram message: "(.+)"$"#)]
async fn send_telegram_message(world: &mut GymWorld, alias: String, message: String) {
    let reply = http::send_chat(world, &alias, &message).await.expect("POST /api/chat failed");
    world.last_reply = Some(reply);
    world.current_user = Some(alias);
}

/// Alternate phrasing: `When tester says "..."`.
#[when(regex = r#"^(\w+) says "(.+)"$"#)]
async fn says(world: &mut GymWorld, alias: String, message: String) {
    let reply = http::send_chat(world, &alias, &message).await.expect("POST /api/chat failed");
    world.last_reply = Some(reply);
    world.current_user = Some(alias);
}

/// Canned phrasing: ask the assistant for the workout status.
#[when(regex = r"^(\w+) asks for the workout status$")]
async fn asks_for_status(world: &mut GymWorld, alias: String) {
    let reply = http::send_chat(world, &alias, "What's the status of my current workout?").await.expect("status query failed");
    world.last_reply = Some(reply);
    world.current_user = Some(alias);
}

/// Canned phrasing: start a new session. The LLM treats vague phrasings like "let's start
/// a session" as conversational; this one is imperative enough to reliably emit a
/// `start_session` action.
#[when(regex = r"^(\w+) starts a new (?:session|workout)$")]
async fn starts_session(world: &mut GymWorld, alias: String) {
    let reply = http::send_chat(world, &alias, "I'm at the gym now and starting my workout. Please open a new session for me.")
        .await
        .expect("start-session message failed");
    world.last_reply = Some(reply);
    world.current_user = Some(alias);
}

/// Canned phrasing: end the current session.
#[when(regex = r"^(\w+) ends the (?:current )?(?:session|workout)$")]
async fn ends_session(world: &mut GymWorld, alias: String) {
    let reply = http::send_chat(world, &alias, "I'm done — please end this workout session.").await.expect("end-session message failed");
    world.last_reply = Some(reply);
    world.current_user = Some(alias);
}
