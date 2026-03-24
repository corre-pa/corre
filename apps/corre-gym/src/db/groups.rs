use anyhow::Context as _;
use rusqlite::params;

use super::database::Database;
use super::models::{AccessLevel, Group, User};
use super::users::row_to_user;

fn row_to_group(row: &rusqlite::Row) -> rusqlite::Result<Group> {
    Ok(Group { id: row.get(0)?, name: row.get(1)?, description: row.get(2)?, created_at: row.get(3)? })
}

impl Database {
    pub fn insert_group(&self, group: &Group) -> anyhow::Result<()> {
        self.conn().execute(
            "INSERT INTO groups (id, name, description, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![group.id, group.name, group.description, group.created_at],
        )?;
        Ok(())
    }

    pub fn get_group(&self, id: &str) -> anyhow::Result<Option<Group>> {
        let mut stmt = self.conn().prepare("SELECT id, name, description, created_at FROM groups WHERE id = ?1")?;
        let mut rows = stmt.query_map(params![id], row_to_group)?;
        rows.next().transpose().context("Failed to read group row")
    }

    pub fn list_groups(&self) -> anyhow::Result<Vec<Group>> {
        let mut stmt = self.conn().prepare("SELECT id, name, description, created_at FROM groups ORDER BY name")?;
        let rows = stmt.query_map([], row_to_group)?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to list groups")
    }

    pub fn update_group(&self, group: &Group) -> anyhow::Result<()> {
        let rows = self
            .conn()
            .execute("UPDATE groups SET name = ?1, description = ?2 WHERE id = ?3", params![group.name, group.description, group.id])?;
        anyhow::ensure!(rows > 0, "Group with id {} not found", group.id);
        Ok(())
    }

    pub fn delete_group(&self, id: &str) -> anyhow::Result<()> {
        let rows = self.conn().execute("DELETE FROM groups WHERE id = ?1", params![id])?;
        anyhow::ensure!(rows > 0, "Group with id {id} not found");
        Ok(())
    }

    pub fn add_member(&self, user_id: &str, group_id: &str, level: AccessLevel) -> anyhow::Result<()> {
        self.conn().execute(
            "INSERT INTO group_members (user_id, group_id, level) VALUES (?1, ?2, ?3)",
            params![user_id, group_id, level.as_str()],
        )?;
        Ok(())
    }

    pub fn remove_member(&self, user_id: &str, group_id: &str) -> anyhow::Result<()> {
        let rows = self.conn().execute("DELETE FROM group_members WHERE user_id = ?1 AND group_id = ?2", params![user_id, group_id])?;
        anyhow::ensure!(rows > 0, "Membership not found for user {user_id} in group {group_id}");
        Ok(())
    }

    pub fn set_member_level(&self, user_id: &str, group_id: &str, level: AccessLevel) -> anyhow::Result<()> {
        let rows = self.conn().execute(
            "UPDATE group_members SET level = ?1 WHERE user_id = ?2 AND group_id = ?3",
            params![level.as_str(), user_id, group_id],
        )?;
        anyhow::ensure!(rows > 0, "Membership not found for user {user_id} in group {group_id}");
        Ok(())
    }

    pub fn list_group_members(&self, group_id: &str) -> anyhow::Result<Vec<(User, AccessLevel)>> {
        let mut stmt = self.conn().prepare(
            "SELECT u.id, u.name, u.telegram_id, u.signal_id, u.timezone, u.created_at, u.updated_at, gm.level \
             FROM users u \
             JOIN group_members gm ON u.id = gm.user_id \
             WHERE gm.group_id = ?1 ORDER BY u.name",
        )?;
        let rows = stmt.query_map(params![group_id], |row| {
            let user = row_to_user(row)?;
            let level = AccessLevel::from_str_loose(&row.get::<_, String>(7)?);
            Ok((user, level))
        })?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to list group members")
    }

    pub fn list_user_groups(&self, user_id: &str) -> anyhow::Result<Vec<(Group, AccessLevel)>> {
        let mut stmt = self.conn().prepare(
            "SELECT g.id, g.name, g.description, g.created_at, gm.level \
             FROM groups g \
             JOIN group_members gm ON g.id = gm.group_id \
             WHERE gm.user_id = ?1 ORDER BY g.name",
        )?;
        let rows = stmt.query_map(params![user_id], |row| {
            let group = row_to_group(row)?;
            let level = AccessLevel::from_str_loose(&row.get::<_, String>(4)?);
            Ok((group, level))
        })?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to list user groups")
    }
}

#[cfg(test)]
mod tests {
    use super::super::models::new_user;
    use super::*;

    fn test_db() -> Database {
        Database::open_in_memory().unwrap()
    }

    fn make_group(name: &str) -> Group {
        Group { id: uuid::Uuid::new_v4().to_string(), name: name.to_string(), description: None, created_at: "2025-01-01 00:00:00".into() }
    }

    #[test]
    fn create_group_and_add_members() {
        let db = test_db();
        let group = make_group("Gym Buddies");
        db.insert_group(&group).unwrap();

        let u1 = new_user("Alice", None, "UTC");
        let u2 = new_user("Bob", None, "UTC");
        db.insert_user(&u1).unwrap();
        db.insert_user(&u2).unwrap();

        db.add_member(&u1.id, &group.id, AccessLevel::Admin).unwrap();
        db.add_member(&u2.id, &group.id, AccessLevel::Read).unwrap();

        let members = db.list_group_members(&group.id).unwrap();
        assert_eq!(members.len(), 2);
    }

    #[test]
    fn remove_member() {
        let db = test_db();
        let group = make_group("Test Group");
        db.insert_group(&group).unwrap();

        let user = new_user("Alice", None, "UTC");
        db.insert_user(&user).unwrap();
        db.add_member(&user.id, &group.id, AccessLevel::Read).unwrap();
        db.remove_member(&user.id, &group.id).unwrap();

        let members = db.list_group_members(&group.id).unwrap();
        assert!(members.is_empty());
    }

    #[test]
    fn set_member_level() {
        let db = test_db();
        let group = make_group("Test Group");
        db.insert_group(&group).unwrap();

        let user = new_user("Alice", None, "UTC");
        db.insert_user(&user).unwrap();
        db.add_member(&user.id, &group.id, AccessLevel::Read).unwrap();
        db.set_member_level(&user.id, &group.id, AccessLevel::Admin).unwrap();

        let members = db.list_group_members(&group.id).unwrap();
        assert_eq!(members[0].1, AccessLevel::Admin);
    }

    #[test]
    fn delete_group_removes_memberships() {
        let db = test_db();
        let group = make_group("Doomed Group");
        db.insert_group(&group).unwrap();

        let user = new_user("Alice", None, "UTC");
        db.insert_user(&user).unwrap();
        db.add_member(&user.id, &group.id, AccessLevel::Read).unwrap();

        db.delete_group(&group.id).unwrap();
        let groups = db.list_user_groups(&user.id).unwrap();
        assert!(groups.is_empty());
    }

    #[test]
    fn list_user_groups() {
        let db = test_db();
        let g1 = make_group("Group A");
        let g2 = make_group("Group B");
        db.insert_group(&g1).unwrap();
        db.insert_group(&g2).unwrap();

        let user = new_user("Alice", None, "UTC");
        db.insert_user(&user).unwrap();
        db.add_member(&user.id, &g1.id, AccessLevel::Write).unwrap();
        db.add_member(&user.id, &g2.id, AccessLevel::Admin).unwrap();

        let groups = db.list_user_groups(&user.id).unwrap();
        assert_eq!(groups.len(), 2);
    }
}
