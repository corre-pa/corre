//! Background steps: register Telegram users, assert a clean instance.

use cucumber::{gherkin::Step, given};

use crate::corre_gym::auth;
use crate::corre_gym::world::GymWorld;

/// `Given telegram user` followed by a key/value data table:
/// ```gherkin
/// Given telegram user
///   | first_name | Tester |
///   | username   | tester |
///   | id         | 123    |
/// ```
///
/// Provisions a DB user and mints a session cookie. The alias used by subsequent steps
/// is the `username` cell if present, otherwise `first_name` (lowercased so feature
/// authors can write either casing).
#[given(expr = "telegram user")]
async fn telegram_user(world: &mut GymWorld, step: &Step) {
    let table = step.table.as_ref().expect("Given telegram user requires a data table");
    let mut first_name: Option<String> = None;
    let mut last_name: Option<String> = None;
    let mut username: Option<String> = None;
    let mut id: Option<i64> = None;
    for row in &table.rows {
        assert!(row.len() == 2, "telegram user table rows must be 2 columns; got {row:?}");
        let key = row[0].trim().to_lowercase();
        let value = row[1].trim().to_string();
        match key.as_str() {
            "first_name" => first_name = Some(value),
            "last_name" => last_name = Some(value),
            "username" => username = Some(value),
            "id" => id = Some(value.parse().expect("id cell must parse as i64")),
            other => panic!("unknown telegram user field `{other}`"),
        }
    }
    let first_name = first_name.expect("telegram user table must include first_name");
    let id = id.expect("telegram user table must include id");
    let display_name = match &last_name {
        Some(last) if !last.is_empty() => format!("{first_name} {last}"),
        _ => first_name.clone(),
    };
    let alias = username.clone().unwrap_or_else(|| first_name.to_lowercase());

    let registered = auth::register_user(world.db(), world.app_state(), &display_name, username.as_deref(), id)
        .await
        .expect("registering telegram user");
    world.users.insert(alias.clone(), registered);
    world.current_user = Some(alias);
}

/// Assert the freshly-spawned server is empty: no users besides the ones the test just
/// registered, no sessions, no sets.
#[given(expr = "a clean corre-gym instance")]
async fn clean_instance(world: &mut GymWorld) {
    let db = world.db().lock().await;
    let user_count: i64 =
        db.conn().query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0)).expect("counting users");
    let session_count: i64 =
        db.conn().query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0)).expect("counting sessions");
    let set_count: i64 =
        db.conn().query_row("SELECT COUNT(*) FROM sets", [], |row| row.get(0)).expect("counting sets");

    assert!(
        user_count as usize == world.users.len(),
        "expected {} registered users, DB has {user_count}",
        world.users.len()
    );
    assert_eq!(session_count, 0, "fresh corre-gym should have no sessions, found {session_count}");
    assert_eq!(set_count, 0, "fresh corre-gym should have no sets, found {set_count}");
}
