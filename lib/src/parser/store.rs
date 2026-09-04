//! The parsers a reader writes, kept between restarts.
//!
//! A parser is the reader's account of how one family of filenames is
//! written. Nothing reconstructs it from a feed, so it is a table a restart
//! must not lose.
//!
//! Keyed by a slug the application fixes when the parser is created, so a
//! rename changes the name a page shows and orphans nothing that points at
//! the parser.

// FIXME: Nothing outside the tests holds a ParserStore. The shared ruleset
// registry is the caller this waits on.
#![allow(dead_code)]

use std::collections::BTreeMap;

use sqlx::{Row, SqlitePool};

use super::{Field, FieldKind, Parser, TitleTest};

/// Adds a parser, or replaces the one already stored under its id.
const UPSERT: &str = "
    INSERT INTO parsers (id, name)
    VALUES (?1, ?2)
    ON CONFLICT (id) DO UPDATE SET name = excluded.name
";

/// Reads every parser by name, which is the order the index lists them.
///
/// The name orders them rather than the id. The id is a slug the reader
/// never sees, and ordering by it leaves a renamed parser where its old name
/// sorted.
const SELECT_PARSERS: &str = "SELECT id, name FROM parsers ORDER BY name";

/// Reads every field of every parser, grouped by parser and in order.
///
/// The fold below walks this once. A query per parser costs a round trip for
/// each instead.
const SELECT_FIELDS: &str = "
    SELECT parser, name, kind, pattern, required, identity, tight
    FROM parser_fields
    ORDER BY parser, position
";

/// Reads every saved test of every parser, grouped by parser and in order.
const SELECT_TESTS: &str = "
    SELECT parser, position, title
    FROM parser_tests
    ORDER BY parser, position
";

/// Reads every expectation of every saved test.
///
/// Ordered by field so a listed test reads the same way twice, which is what
/// the round-trip comparison rests on.
const SELECT_TEST_VALUES: &str = "
    SELECT parser, position, field, expected
    FROM parser_test_values
    ORDER BY parser, position, field
";

/// The stored parsers, read and written through one pool.
pub(crate) struct ParserStore {
    pool: SqlitePool,
}

impl ParserStore {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Returns every stored parser with its fields and saved tests, ordered
    /// by name.
    ///
    /// # Errors
    ///
    /// Returns a decode failure when a row names a field kind this build
    /// does not know. Every stored value was one when it was written, so the
    /// row is corrupt rather than merely unexpected.
    pub(crate) async fn list(&self) -> Result<Vec<Parser>, sqlx::Error> {
        let mut parsers = sqlx::query_as::<_, (String, String)>(SELECT_PARSERS)
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(|(id, name)| Parser {
                id,
                name,
                fields: Vec::new(),
                tests: Vec::new(),
            })
            .collect::<Vec<_>>();

        for row in sqlx::query(SELECT_FIELDS).fetch_all(&self.pool).await? {
            let owner: String = row.try_get("parser")?;

            // A field outlives its parser row only if the foreign key falls,
            // so a miss here reports nothing.
            let Some(parser) = parsers.iter_mut().find(|parser| parser.id == owner) else {
                continue;
            };

            parser.fields.push(field(&row)?);
        }

        for row in sqlx::query(SELECT_TESTS).fetch_all(&self.pool).await? {
            let owner: String = row.try_get("parser")?;

            let Some(parser) = parsers.iter_mut().find(|parser| parser.id == owner) else {
                continue;
            };

            parser.tests.push(TitleTest {
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
            let owner: String = row.try_get("parser")?;
            let position: i64 = row.try_get("position")?;

            let Some(test) = parsers
                .iter_mut()
                .find(|parser| parser.id == owner)
                .and_then(|parser| parser.tests.get_mut(usize::try_from(position).ok()?))
            else {
                continue;
            };

            test.expected
                .insert(row.try_get("field")?, row.try_get("expected")?);
        }

        Ok(parsers)
    }

    /// Writes `parser` with its fields and saved tests, replacing whatever
    /// was stored.
    ///
    /// Both lists are deleted and reinserted rather than updated in place,
    /// because a save drops a row as readily as it changes one. The whole
    /// write is one transaction, so a failure part way leaves the parser as
    /// it was rather than half rewritten.
    pub(crate) async fn upsert(&self, parser: &Parser) -> Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;

        sqlx::query(UPSERT)
            .bind(&parser.id)
            .bind(&parser.name)
            .execute(&mut *tx)
            .await?;

        sqlx::query("DELETE FROM parser_fields WHERE parser = ?1")
            .bind(&parser.id)
            .execute(&mut *tx)
            .await?;

        for (position, field) in parser.fields.iter().enumerate() {
            sqlx::query(
                "INSERT INTO parser_fields
                    (parser, position, name, kind, pattern, required, identity, tight)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )
            .bind(&parser.id)
            .bind(position as i64)
            .bind(&field.name)
            .bind(field.kind.label())
            .bind(field.pattern.as_deref())
            .bind(field.required)
            .bind(field.identity)
            .bind(field.tight)
            .execute(&mut *tx)
            .await?;
        }

        // The values cascade from the tests, so one delete clears both.
        sqlx::query("DELETE FROM parser_tests WHERE parser = ?1")
            .bind(&parser.id)
            .execute(&mut *tx)
            .await?;

        for (position, test) in parser.tests.iter().enumerate() {
            let position = position as i64;

            sqlx::query("INSERT INTO parser_tests (parser, position, title) VALUES (?1, ?2, ?3)")
                .bind(&parser.id)
                .bind(position)
                .bind(&test.title)
                .execute(&mut *tx)
                .await?;

            for (field, expected) in &test.expected {
                sqlx::query(
                    "INSERT INTO parser_test_values (parser, position, field, expected)
                     VALUES (?1, ?2, ?3, ?4)",
                )
                .bind(&parser.id)
                .bind(position)
                .bind(field)
                .bind(expected)
                .execute(&mut *tx)
                .await?;
            }
        }

        tx.commit().await
    }

    /// Removes the parser `id` with its fields and tests, and reports whether
    /// one was there.
    pub(crate) async fn remove(&self, id: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM parsers WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() == 1)
    }
}

/// Rebuilds one field from its row.
///
/// The kind is stored as the same text the editor's form posts, so one
/// vocabulary serves the form and the table.
///
/// The ruleset store reads its own field rows through this too. Both tables
/// carry the same columns, because a ruleset still declares fields of its
/// own until every one of them runs on a parser.
pub(crate) fn field(row: &sqlx::sqlite::SqliteRow) -> Result<Field, sqlx::Error> {
    let kind: String = row.try_get("kind")?;

    Ok(Field {
        name: row.try_get("name")?,
        kind: FieldKind::from_label(&kind)
            .ok_or_else(|| sqlx::Error::decode(format!("unknown field kind {kind}")))?,
        pattern: row.try_get("pattern")?,
        required: row.try_get("required")?,
        tight: row.try_get("tight")?,
        identity: row.try_get("identity")?,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use sqlx::SqlitePool;

    use super::{Field, FieldKind, Parser, ParserStore, TitleTest};

    fn text_field(name: &str, pattern: &str) -> Field {
        Field {
            name: name.to_owned(),
            kind: FieldKind::Text,
            pattern: Some(pattern.to_owned()),
            required: true,
            tight: false,
            identity: false,
        }
    }

    fn parser(id: &str, fields: Vec<Field>) -> Parser {
        Parser {
            id: id.to_owned(),
            name: id.to_owned(),
            fields,
            tests: Vec::new(),
        }
    }

    /// Two fields, whose order the position column keeps, and one saved test
    /// naming both of them.
    fn series() -> Parser {
        Parser {
            tests: vec![TitleTest {
                title: "The.Hollow.Meridian.S04E06".to_owned(),
                expected: BTreeMap::from([
                    ("show".to_owned(), "the hollow meridian".to_owned()),
                    ("season".to_owned(), "4".to_owned()),
                ]),
            }],
            ..parser(
                "series",
                vec![
                    Field {
                        tight: true,
                        ..text_field("show", r"^(?<show>\w+)")
                    },
                    text_field("season", r"\.S(?<season>\d+)"),
                ],
            )
        }
    }

    #[sqlx::test]
    async fn list_of_a_fresh_database_is_empty(pool: SqlitePool) {
        assert_eq!(
            ParserStore::new(pool).list().await.expect("list"),
            Vec::new()
        );
    }

    #[sqlx::test]
    async fn upsert_then_list_round_trips_each_parser(pool: SqlitePool) {
        let store = ParserStore::new(pool);
        store.upsert(&series()).await.expect("the first");
        store
            .upsert(&parser("archive", vec![text_field("year", r"\.\d{4}")]))
            .await
            .expect("the second");

        assert_eq!(
            store.list().await.expect("list"),
            vec![
                parser("archive", vec![text_field("year", r"\.\d{4}")]),
                series(),
            ],
            "ordered by name, each with its fields in position order"
        );
    }

    #[sqlx::test]
    async fn a_second_upsert_replaces_the_fields_and_the_tests(pool: SqlitePool) {
        let store = ParserStore::new(pool);
        store.upsert(&series()).await.expect("the first");

        let edited = parser("series", vec![text_field("title", "^.")]);
        store.upsert(&edited).await.expect("the edit");

        assert_eq!(
            store.list().await.expect("list"),
            vec![edited],
            "a dropped field and a dropped test both go"
        );
    }

    #[sqlx::test]
    async fn remove_reports_the_row_and_cascades_the_tests(pool: SqlitePool) {
        let store = ParserStore::new(pool.clone());
        store.upsert(&series()).await.expect("the first");

        assert!(
            store.remove("series").await.expect("remove"),
            "a stored row"
        );
        assert!(
            !store.remove("series").await.expect("remove"),
            "and nothing the second time"
        );

        let orphans: i64 = sqlx::query_scalar("SELECT count(*) FROM parser_test_values")
            .fetch_one(&pool)
            .await
            .expect("count");

        assert_eq!(orphans, 0, "the expectations leave with the parser");
    }
}
