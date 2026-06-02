//! Rest-timer assertions on `/api/chat` replies.

use cucumber::then;

use crate::corre_gym::world::GymWorld;

#[then(regex = r"^the reply queues a (\d+)s rest timer$")]
async fn reply_queues_rest_timer(world: &mut GymWorld, expected_secs: u32) {
    let reply = world.last_reply.as_ref().expect("no /api/chat reply captured");
    let timer = reply.rest_timer.as_ref().unwrap_or_else(|| panic!("reply has no rest_timer field; reply text: {:?}", reply.text));
    assert_eq!(
        timer.duration_secs, expected_secs,
        "expected {expected_secs}s rest timer, got {}s. Reply text: {:?}", timer.duration_secs, reply.text
    );
}

#[then(regex = r#"^the reply queues a (\d+)s rest timer for "(.+)"$"#)]
async fn reply_queues_rest_timer_for(world: &mut GymWorld, expected_secs: u32, expected_exercise: String) {
    let reply = world.last_reply.as_ref().expect("no /api/chat reply captured");
    let timer = reply.rest_timer.as_ref().unwrap_or_else(|| panic!("reply has no rest_timer field; reply text: {:?}", reply.text));
    assert_eq!(timer.duration_secs, expected_secs, "expected {expected_secs}s rest timer, got {}s", timer.duration_secs);
    assert_eq!(timer.exercise_name, expected_exercise, "expected timer for {expected_exercise:?}, got {:?}", timer.exercise_name);
}

#[then("the rest timer is a superset timer")]
async fn rest_timer_is_superset(world: &mut GymWorld) {
    let reply = world.last_reply.as_ref().expect("no /api/chat reply captured");
    let timer = reply.rest_timer.as_ref().expect("reply has no rest_timer");
    assert!(timer.is_superset, "expected is_superset=true on timer");
}

#[then("the reply queues no rest timer")]
async fn reply_queues_no_rest_timer(world: &mut GymWorld) {
    let reply = world.last_reply.as_ref().expect("no /api/chat reply captured");
    assert!(reply.rest_timer.is_none(), "expected no rest timer, found {:?}", reply.rest_timer);
}

#[then("the reply cancels any pending rest timer")]
async fn reply_cancels_pending_rest_timer(world: &mut GymWorld) {
    let reply = world.last_reply.as_ref().expect("no /api/chat reply captured");
    assert!(reply.cancel_rest_timer, "expected cancel_rest_timer=true on reply");
}
