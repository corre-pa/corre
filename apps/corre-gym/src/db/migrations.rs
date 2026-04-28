use std::sync::LazyLock;

use include_dir::{Dir, include_dir};
use rusqlite_migration::Migrations;

static MIGRATIONS_DIR: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/migrations");

pub static MIGRATIONS: LazyLock<Migrations<'static>> =
    LazyLock::new(|| Migrations::from_directory(&MIGRATIONS_DIR).expect("invalid migrations directory"));

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::*;

    #[test]
    fn migrations_valid() {
        MIGRATIONS.validate().expect("migrations failed validation");
    }

    #[test]
    fn migrations_round_trip_up_then_down() {
        let mut conn = Connection::open_in_memory().unwrap();
        MIGRATIONS.to_latest(&mut conn).expect("up to latest failed");
        let after_up: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(after_up, 2);

        MIGRATIONS.to_version(&mut conn, 0).expect("down to 0 failed");
        let after_down: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(after_down, 0);

        let table_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(table_count, 0, "all app tables should be dropped");

        MIGRATIONS.to_latest(&mut conn).expect("re-apply up failed");
        let after_redo: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(after_redo, 2);
    }
}
