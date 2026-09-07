//! The searches a reader writes, kept between restarts.
//!
//! A search is one of the two things in this application the reader authors,
//! the parser being the other. Every other table records what a feed or a
//! client reported. This one holds what the reader decided, so it is a table
//! a restart must not lose.
//!
//! Keyed by a slug the application fixes when the search is created.
//! `grab_searches.search` carries that slug, so a rename changes the name a
//! page shows and orphans nothing.

// FIXME: Nothing outside the tests holds a SearchStore, so every item here
// is unused. The shared search registry is the caller this waits on.
#![allow(dead_code)]

use std::collections::BTreeMap;

use sqlx::{Row, SqlitePool};

use super::{Condition, Op, Search};
use crate::parser::TitleTest;

/// Adds a search, or replaces the one already stored under its id.
///
/// `enabled` keeps its stored value on conflict. The switch is the reader's
/// runtime decision about a search, not part of the rules they edit, so
/// saving an edit never turns a running search off.
const UPSERT: &str = "
    INSERT INTO searches (id, name, parser, enabled)
    VALUES (?1, ?2, ?3, ?4)
    ON CONFLICT (id) DO UPDATE SET
        name = excluded.name,
        parser = excluded.parser
";

/// Reads every search by name, which is the order the admin index lists them.
///
/// The name orders them rather than the id. The id is a slug the reader
/// never sees, and ordering by it leaves a renamed search where its old
/// name sorted.
const SELECT_SEARCHES: &str = "SELECT id, name, parser, enabled FROM searches ORDER BY name";

/// Reads every condition of every search, grouped by search and in order.
const SELECT_CONDITIONS: &str = "
    SELECT search, field, op, value
    FROM search_conditions
    ORDER BY search, position
";

/// Reads every saved test of every search, grouped by search and in order.
const SELECT_TESTS: &str = "
    SELECT search, position, title
    FROM search_tests
    ORDER BY search, position
";

/// Reads every expectation of every saved test.
///
/// Ordered by field so a listed test reads the same way twice, which is what
/// the round-trip comparison rests on.
const SELECT_TEST_VALUES: &str = "
    SELECT search, position, field, expected
    FROM search_test_values
    ORDER BY search, position, field
";

/// The stored searches, read and written through one pool.
pub(crate) struct SearchStore {
    pool: SqlitePool,
}

impl SearchStore {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Returns every stored search with its conditions and saved tests,
    /// ordered by name.
    ///
    /// # Errors
    ///
    /// Returns a decode failure when a row names a condition operator this
    /// build does not know. Every stored value was one when it was written,
    /// so the row is corrupt rather than merely unexpected.
    pub(crate) async fn list(&self) -> Result<Vec<Search>, sqlx::Error> {
        let mut searches = sqlx::query_as::<_, (String, String, String, bool)>(SELECT_SEARCHES)
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(|(id, name, parser, enabled)| Search {
                id,
                name,
                enabled,
                parser,
                conditions: Vec::new(),
                tests: Vec::new(),
            })
            .collect::<Vec<_>>();

        for row in sqlx::query(SELECT_CONDITIONS).fetch_all(&self.pool).await? {
            let owner: String = row.try_get("search")?;

            let Some(search) = searches.iter_mut().find(|search| search.id == owner) else {
                continue;
            };

            let op: String = row.try_get("op")?;

            search.conditions.push(Condition {
                field: row.try_get("field")?,
                op: Op::from_label(&op)
                    .ok_or_else(|| sqlx::Error::decode(format!("unknown condition op {op}")))?,
                value: row.try_get("value")?,
            });
        }

        for row in sqlx::query(SELECT_TESTS).fetch_all(&self.pool).await? {
            let owner: String = row.try_get("search")?;

            let Some(search) = searches.iter_mut().find(|search| search.id == owner) else {
                continue;
            };

            search.tests.push(TitleTest {
                title: row.try_get("title")?,
                expected: BTreeMap::new(),
            });
        }

        // The tests land above in position order, contiguous from zero,
        // because a save rewrites the whole list and enumerates it. That is
        // what lets a value find its test by index.
        for row in sqlx::query(SELECT_TEST_VALUES)
            .fetch_all(&self.pool)
            .await?
        {
            let owner: String = row.try_get("search")?;
            let position: i64 = row.try_get("position")?;

            let Some(test) = searches
                .iter_mut()
                .find(|search| search.id == owner)
                .and_then(|search| search.tests.get_mut(usize::try_from(position).ok()?))
            else {
                continue;
            };

            test.expected
                .insert(row.try_get("field")?, row.try_get("expected")?);
        }

        Ok(searches)
    }

    /// Writes `search` with its conditions and saved tests, replacing
    /// whatever was stored.
    ///
    /// Every list is deleted and reinserted rather than updated in place,
    /// because a save drops a row as readily as it changes one. The whole
    /// write is one transaction, so a failure part way leaves the search as
    /// it was rather than half rewritten.
    ///
    /// A search already stored keeps its enabled state. Saving an edit is
    /// not a request to start or stop the search.
    pub(crate) async fn upsert(&self, search: &Search) -> Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;

        sqlx::query(UPSERT)
            .bind(&search.id)
            .bind(&search.name)
            .bind(&search.parser)
            .bind(search.enabled)
            .execute(&mut *tx)
            .await?;

        sqlx::query("DELETE FROM search_conditions WHERE search = ?1")
            .bind(&search.id)
            .execute(&mut *tx)
            .await?;

        for (position, condition) in search.conditions.iter().enumerate() {
            sqlx::query(
                "INSERT INTO search_conditions (search, position, field, op, value)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .bind(&search.id)
            .bind(position as i64)
            .bind(&condition.field)
            .bind(condition.op.label())
            .bind(&condition.value)
            .execute(&mut *tx)
            .await?;
        }

        // The values cascade from the tests, so one delete clears both.
        sqlx::query("DELETE FROM search_tests WHERE search = ?1")
            .bind(&search.id)
            .execute(&mut *tx)
            .await?;

        for (position, test) in search.tests.iter().enumerate() {
            let position = position as i64;

            sqlx::query("INSERT INTO search_tests (search, position, title) VALUES (?1, ?2, ?3)")
                .bind(&search.id)
                .bind(position)
                .bind(&test.title)
                .execute(&mut *tx)
                .await?;

            for (field, expected) in &test.expected {
                sqlx::query(
                    "INSERT INTO search_test_values (search, position, field, expected)
                     VALUES (?1, ?2, ?3, ?4)",
                )
                .bind(&search.id)
                .bind(position)
                .bind(field)
                .bind(expected)
                .execute(&mut *tx)
                .await?;
            }
        }

        tx.commit().await
    }

    /// Removes the search `id` with its conditions and tests, and reports
    /// whether one was there.
    pub(crate) async fn remove(&self, id: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM searches WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() == 1)
    }

    /// Switches the search `id` on or off, and reports whether one was there.
    pub(crate) async fn set_enabled(&self, id: &str, enabled: bool) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("UPDATE searches SET enabled = ?2 WHERE id = ?1")
            .bind(id)
            .bind(enabled)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() == 1)
    }
}
#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use sqlx::SqlitePool;

    use super::{Condition, Op, Search, SearchStore};
    use crate::parser::store::ParserStore;
    use crate::parser::{Field, FieldKind, Parser, TitleTest};

    /// The parser every search below reads with.
    ///
    /// Most tests write it first, which mirrors a reader who wrote a parser
    /// of their own before the search that reads with it.
    fn parser() -> Parser {
        Parser {
            id: "series".to_owned(),
            name: "Series".to_owned(),
            fields: vec![Field {
                name: "season".to_owned(),
                kind: FieldKind::Season,
                pattern: None,
                required: true,
                tight: true,
                identity: true,
            }],
            tests: Vec::new(),
            built_in: false,
        }
    }

    fn search(id: &str) -> Search {
        Search {
            id: id.to_owned(),
            name: id.to_owned(),
            enabled: false,
            parser: "series".to_owned(),
            conditions: Vec::new(),
            tests: Vec::new(),
        }
    }

    /// One condition and one saved test, so both lists cross the table.
    fn narrowed() -> Search {
        Search {
            conditions: vec![Condition {
                field: "season".to_owned(),
                op: Op::Equals,
                value: "4".to_owned(),
            }],
            tests: vec![TitleTest {
                title: "The.Hollow.Meridian.S04E06".to_owned(),
                expected: BTreeMap::from([
                    ("show".to_owned(), "the hollow meridian".to_owned()),
                    ("season".to_owned(), "4".to_owned()),
                ]),
            }],
            ..search("hollow")
        }
    }

    /// A store over `pool` with the parser already written.
    async fn stored(pool: &SqlitePool) -> SearchStore {
        ParserStore::new(pool.clone())
            .upsert(&parser())
            .await
            .expect("the parser the searches read with");

        SearchStore::new(pool.clone())
    }

    #[sqlx::test]
    async fn list_of_a_fresh_database_is_empty(pool: SqlitePool) {
        assert_eq!(
            SearchStore::new(pool).list().await.expect("list"),
            Vec::new()
        );
    }

    #[sqlx::test]
    async fn upsert_stores_a_parser_no_row_stands_behind(pool: SqlitePool) {
        let store = SearchStore::new(pool.clone());

        store
            .upsert(&search("hollow"))
            .await
            .expect("no parsers row is needed");

        assert_eq!(
            store.list().await.expect("list"),
            vec![search("hollow")],
            "a parser the binary carries has no row, and a search on one still stores"
        );
    }

    #[sqlx::test]
    async fn upsert_then_list_round_trips_each_search(pool: SqlitePool) {
        let store = stored(&pool).await;
        store.upsert(&narrowed()).await.expect("the narrowed one");
        store.upsert(&search("archive")).await.expect("the other");

        assert_eq!(
            store.list().await.expect("list"),
            vec![search("archive"), narrowed()],
            "ordered by name, each with its conditions and tests"
        );
    }

    #[sqlx::test]
    async fn a_second_upsert_replaces_the_lists_and_keeps_enabled(pool: SqlitePool) {
        let store = stored(&pool).await;
        store.upsert(&narrowed()).await.expect("the first");
        store.set_enabled("hollow", true).await.expect("enable");

        let mut edited = search("hollow");
        store.upsert(&edited).await.expect("the edit");
        edited.enabled = true;

        assert_eq!(
            store.list().await.expect("list"),
            vec![edited],
            "a dropped condition and a dropped test both go, and the switch is not part of \
             the edit"
        );
    }

    #[sqlx::test]
    async fn remove_reports_the_row_and_cascades_the_tests_and_conditions(pool: SqlitePool) {
        let store = stored(&pool).await;
        store.upsert(&narrowed()).await.expect("the first");

        assert!(
            store.remove("hollow").await.expect("remove"),
            "a stored row"
        );
        assert!(
            !store.remove("hollow").await.expect("remove again"),
            "nothing left to remove"
        );
        assert_eq!(store.list().await.expect("list"), Vec::new());

        // The values hang off the tests, which hang off the search, so this
        // is the far end of the chain and the one a single delete has to
        // reach.
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM search_test_values")
                .fetch_one(&pool)
                .await
                .expect("count"),
            0,
            "and the expectations go with the tests that named them"
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM search_conditions")
                .fetch_one(&pool)
                .await
                .expect("count"),
            0,
            "and so do the conditions"
        );
    }

    #[sqlx::test]
    async fn set_enabled_flips_and_reports(pool: SqlitePool) {
        let store = stored(&pool).await;
        store.upsert(&narrowed()).await.expect("the first");

        assert!(
            store.set_enabled("hollow", true).await.expect("enable"),
            "a stored row"
        );
        assert!(
            store.list().await.expect("list")[0].enabled,
            "the flip reaches the table"
        );
        assert!(
            !store.set_enabled("absent", true).await.expect("unknown"),
            "no row to flip"
        );
    }
}
