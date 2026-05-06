//! Health-entry assertions. Triggered by free-form chat messages elsewhere; this file
//! provides the matchers for `Then a health entry is recorded:` etc.

use cucumber::{gherkin::Step, then};

use corre_gym::db::HealthEntryType;

use crate::corre_gym::assertions::{self, table_to_map};
use crate::corre_gym::world::GymWorld;

#[then(expr = "the following health entry is recorded:")]
async fn health_entry_recorded(world: &mut GymWorld, step: &Step) {
    let alias = world.current_user.clone().expect("no current user");
    let user_id = world.user(&alias).expect("user lookup").user.id;
    let table = step.table.as_ref().expect("`the following health entry is recorded:` requires a data table");
    let map = table_to_map(table).expect("parsing health table");

    let entry = assertions::last_active_health_entry(world.db(), user_id)
        .await
        .expect("loading health entries")
        .expect("expected an active health entry, but none exist");

    for (key, value) in &map {
        match key.as_str() {
            "entry_type" => {
                let expected = HealthEntryType::from_str_loose(value);
                assert_eq!(entry.entry_type, expected, "entry_type mismatch");
            }
            "body_part" => {
                let actual = entry.body_part.as_deref().unwrap_or("").to_lowercase();
                let needle = value.to_lowercase();
                assert!(actual.contains(&needle), "body_part mismatch: expected substring `{value}`, got `{}`", actual);
            }
            "severity" => {
                let actual = entry.severity.to_lowercase();
                let needle = value.to_lowercase();
                assert!(actual.contains(&needle), "severity mismatch: expected `{value}`, got `{}`", entry.severity);
            }
            "description" => {
                let actual = entry.description.to_lowercase();
                let needle = value.to_lowercase();
                assert!(actual.contains(&needle), "description mismatch: expected substring `{value}`, got `{}`", entry.description);
            }
            other => panic!("unknown health entry field `{other}`"),
        }
    }
}
