//! What the application keeps between restarts.
//!
//! The feed registry lives in memory and empties on a restart, but the items
//! it fetched do not. Every item any feed returned is written here raw, so
//! the release parser reads the whole history rather than only what the last
//! poll happened to catch.

use sqlx::SqlitePool;
use sqlx::migrate::MigrateError;

/// Brings `pool` up to the current schema.
///
/// Applies only the migrations a database lacks, so a call on every start is
/// correct and cheap. The SQL is embedded at compile time from the library's
/// `migrations` directory, so a deployment ships the binary alone.
pub async fn migrate(pool: &SqlitePool) -> Result<(), MigrateError> {
    sqlx::migrate!().run(pool).await
}

#[cfg(test)]
mod tests {
    use sqlx::SqlitePool;

    use super::migrate;

    /// `#[sqlx::test]` has already applied the same migrations to `pool`, so
    /// this runs them a second time. Reading the table afterwards proves both
    /// that it exists and that the repeat run changed nothing.
    #[sqlx::test]
    async fn migrate_is_idempotent(pool: SqlitePool) {
        migrate(&pool)
            .await
            .expect("a repeat migration is harmless");

        let items = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM feed_items")
            .fetch_one(&pool)
            .await
            .expect("the feed_items table exists");

        assert_eq!(items, 0, "a migration seeds no rows");
    }
}
