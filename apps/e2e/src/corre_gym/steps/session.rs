//! Session-state assertions.

use cucumber::then;

use crate::corre_gym::assertions;
use crate::corre_gym::world::GymWorld;

#[then(expr = "there is no active session")]
async fn no_active_session(world: &mut GymWorld) {
    let alias = world.current_user.clone().expect("no current user — call a When step first");
    let user = world.user(&alias).expect("user lookup").user.clone();
    let active = assertions::active_session(world.db(), user.id).await.expect("checking active session");
    assert!(active.is_none(), "expected no active session for {}, found session id {:?}", user.name, active.map(|s| s.id));
}

#[then(expr = "there is an active session")]
async fn yes_active_session(world: &mut GymWorld) {
    let alias = world.current_user.clone().expect("no current user — call a When step first");
    let user = world.user(&alias).expect("user lookup").user.clone();
    let active = assertions::active_session(world.db(), user.id).await.expect("checking active session");
    assert!(
        active.is_some(),
        "expected an active session for {}, but get_active_session returned None. Last assistant reply: {:?}",
        user.name,
        world.last_reply.as_ref().map(|r| r.text.as_str()),
    );
}
