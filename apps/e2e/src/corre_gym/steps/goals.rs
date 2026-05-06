//! Goal assertions.

use cucumber::{gherkin::Step, then};

use crate::corre_gym::assertions::{resolve_exercise_type, table_to_map};
use crate::corre_gym::world::GymWorld;

#[then(expr = "the following goal is recorded:")]
async fn goal_recorded(world: &mut GymWorld, step: &Step) {
    let alias = world.current_user.clone().expect("no current user");
    let user_id = world.user(&alias).expect("user lookup").user.id;
    let table = step.table.as_ref().expect("`the following goal is recorded:` requires a data table");
    let map = table_to_map(table).expect("parsing goal table");

    let goals = {
        let db = world.db().lock().await;
        db.list_goals_in_period(user_id, "1970-01-01", "9999-12-31").expect("listing goals")
    };
    let latest = goals.into_iter().max_by(|a, b| a.created_at.cmp(&b.created_at)).expect("no goals recorded");

    for (key, value) in &map {
        match key.as_str() {
            "exercise_type" => {
                let expected = resolve_exercise_type(world.db(), value).await.expect("resolving exercise type");
                assert_eq!(
                    latest.exercise_type_id, expected.id,
                    "goal exercise_type_id mismatch: expected `{}` (id {}), got id {}",
                    expected.name, expected.id, latest.exercise_type_id
                );
            }
            "target_value" => {
                let expected: f64 = value.parse().expect("target_value must parse as f64");
                assert!(
                    (latest.target_value - expected).abs() < 0.01,
                    "target_value mismatch: expected {expected}, got {}",
                    latest.target_value
                );
            }
            "end_date" => {
                let expected = value.trim();
                let actual = latest.end_date.as_deref().unwrap_or("");
                assert!(actual.starts_with(expected), "end_date mismatch: expected prefix `{expected}`, got `{actual}`");
            }
            other => panic!("unknown goal field `{other}`"),
        }
    }
}
