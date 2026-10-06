//! Ordered schema upgrades. Each SQL file commits its own version atomically.
use rusqlite::Connection;

use crate::{Error, Result};

const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/001-index.sql"),
    include_str!("../migrations/002-extraction-cache-and-jobs.sql"),
    include_str!("../migrations/003-extraction-retries.sql"),
    include_str!("../migrations/004-queue-indexes.sql"),
];

pub(crate) fn apply(connection: &Connection) -> Result<()> {
    let current: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if current < 0 || current as usize > MIGRATIONS.len() {
        return Err(Error::InvalidInput(
            "database belongs to a newer Lenscribe version".into(),
        ));
    }
    for migration in &MIGRATIONS[current as usize..] {
        connection.execute_batch(migration)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_a_newer_schema_does_not_modify_it() {
        let connection = Connection::open_in_memory().unwrap();
        let future_version = MIGRATIONS.len() as i64 + 1;
        connection
            .pragma_update(None, "user_version", future_version)
            .unwrap();
        assert!(apply(&connection).is_err());
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, future_version);
        let tables: i64 = connection
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tables, 0);
    }
}
