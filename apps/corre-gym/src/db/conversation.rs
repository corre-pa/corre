use anyhow::Context as _;
use rusqlite::params;

use super::database::Database;
use super::models::{ConversationMessage, ConversationRole};

fn row_to_message(row: &rusqlite::Row) -> rusqlite::Result<ConversationMessage> {
    Ok(ConversationMessage {
        id: row.get(0)?,
        user_id: row.get(1)?,
        platform: row.get(2)?,
        role: ConversationRole::from_str_loose(&row.get::<_, String>(3)?),
        content: row.get(4)?,
        timestamp: row.get(5)?,
    })
}

const SELECT_MSG: &str = "SELECT id, user_id, platform, role, content, timestamp FROM conversation_history";

impl Database {
    pub fn insert_message(&self, msg: &ConversationMessage) -> anyhow::Result<()> {
        self.conn().execute(
            "INSERT INTO conversation_history (id, user_id, platform, role, content, timestamp) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![msg.id, msg.user_id, msg.platform, msg.role.as_str(), msg.content, msg.timestamp],
        )?;
        tracing::debug!(id = %msg.id, role = %msg.role.as_str(), platform = %msg.platform, "DB: inserted message");
        Ok(())
    }

    pub fn get_recent_messages(&self, user_id: &str, limit: usize) -> anyhow::Result<Vec<ConversationMessage>> {
        let sql = format!("{SELECT_MSG} WHERE user_id = ?1 ORDER BY timestamp DESC LIMIT ?2");
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map(params![user_id, limit as i64], row_to_message)?;
        let mut msgs: Vec<_> = rows.collect::<Result<Vec<_>, _>>().context("Failed to get recent messages")?;
        msgs.reverse(); // chronological order
        Ok(msgs)
    }

    pub fn get_recent_messages_for_platform(
        &self,
        user_id: &str,
        platform: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<ConversationMessage>> {
        let sql = format!("{SELECT_MSG} WHERE user_id = ?1 AND platform = ?2 ORDER BY timestamp DESC LIMIT ?3");
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map(params![user_id, platform, limit as i64], row_to_message)?;
        let mut msgs: Vec<_> = rows.collect::<Result<Vec<_>, _>>().context("Failed to get recent messages for platform")?;
        msgs.reverse();
        Ok(msgs)
    }

    pub fn prune_old_messages(&self, user_id: &str, keep_last: usize) -> anyhow::Result<usize> {
        let deleted = self.conn().execute(
            "DELETE FROM conversation_history WHERE user_id = ?1 AND id NOT IN \
             (SELECT id FROM conversation_history WHERE user_id = ?1 ORDER BY timestamp DESC LIMIT ?2)",
            params![user_id, keep_last as i64],
        )?;
        if deleted > 0 {
            tracing::debug!(user_id = %user_id, deleted = %deleted, "DB: pruned old messages");
        }
        Ok(deleted)
    }
}

#[cfg(test)]
mod tests {
    use super::super::models::{new_conversation_message, new_user};
    use super::*;

    fn test_db() -> Database {
        Database::open_in_memory().unwrap()
    }

    #[test]
    fn insert_and_get_recent() {
        let db = test_db();
        let user = new_user("Tester", None, "UTC");
        db.insert_user(&user).unwrap();

        let m1 = new_conversation_message(&user.id, "telegram", ConversationRole::User, "Hello");
        let m2 = new_conversation_message(&user.id, "telegram", ConversationRole::Assistant, "Hi there!");
        db.insert_message(&m1).unwrap();
        db.insert_message(&m2).unwrap();

        let msgs = db.get_recent_messages(&user.id, 10).unwrap();
        assert_eq!(msgs.len(), 2);
        assert!(msgs.iter().any(|m| m.role == ConversationRole::User));
        assert!(msgs.iter().any(|m| m.role == ConversationRole::Assistant));
    }

    #[test]
    fn get_recent_by_platform() {
        let db = test_db();
        let user = new_user("Tester", None, "UTC");
        db.insert_user(&user).unwrap();

        let m1 = new_conversation_message(&user.id, "telegram", ConversationRole::User, "From TG");
        let m2 = new_conversation_message(&user.id, "web", ConversationRole::User, "From Web");
        db.insert_message(&m1).unwrap();
        db.insert_message(&m2).unwrap();

        let msgs = db.get_recent_messages_for_platform(&user.id, "telegram", 10).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "From TG");
    }

    #[test]
    fn prune_old_messages() {
        let db = test_db();
        let user = new_user("Tester", None, "UTC");
        db.insert_user(&user).unwrap();

        for i in 0..10 {
            let msg = new_conversation_message(&user.id, "telegram", ConversationRole::User, &format!("Message {i}"));
            db.insert_message(&msg).unwrap();
        }

        let deleted = db.prune_old_messages(&user.id, 3).unwrap();
        assert_eq!(deleted, 7);

        let remaining = db.get_recent_messages(&user.id, 100).unwrap();
        assert_eq!(remaining.len(), 3);
    }
}
