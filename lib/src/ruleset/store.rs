//! The rulesets a reader writes, kept between restarts.
//!
//! A ruleset is the one thing in this application the reader authors. Every
//! other table records what a feed or a client reported. This one holds what
//! the reader decided, so it is the table a restart must not lose.
//!
//! Keyed by a slug the application fixes when the ruleset is created.
//! `library.identity` and `grab_rulesets.ruleset` both carry that slug, so a
//! rename changes the name a page shows and orphans nothing.

// FIXME: Nothing outside the tests holds a RulesetStore, so every item here
// is unused. The shared ruleset registry is the caller this waits on.
#![allow(dead_code)]

use std::collections::BTreeMap;

use sqlx::{Row, SqlitePool};

use super::{Field, FieldKind, Ruleset, RulesetTest};

/// Adds a ruleset, or replaces the one already stored under its id.
///
/// `enabled` keeps its stored value on conflict. The switch is the reader's
/// runtime decision about a ruleset, not part of the rules they edit, so
/// saving an edit never turns a running ruleset off.
const UPSERT: &str = "
    INSERT INTO rulesets (id, name, based_on, template, enabled)
    VALUES (?1, ?2, ?3, ?4, ?5)
    ON CONFLICT (id) DO UPDATE SET
        name = excluded.name,
        based_on = excluded.based_on,
        template = excluded.template
";

/// Reads every ruleset by name, which is the order the admin index lists them.
///
/// The name orders them rather than the id. The id is a slug the reader
/// never sees, and ordering by it leaves a renamed ruleset where its old
/// name sorted.
const SELECT_RULESETS: &str =
    "SELECT id, name, based_on, template, enabled FROM rulesets ORDER BY name";

/// Reads every field of every ruleset, grouped by ruleset and in order.
///
/// The fold below walks this once. A query per ruleset costs a round trip
/// for each instead.
const SELECT_FIELDS: &str = "
    SELECT ruleset, name, kind, pattern, required, identity, tight
    FROM ruleset_fields
    ORDER BY ruleset, position
";

/// Reads every saved test of every ruleset, grouped by ruleset and in order.
const SELECT_TESTS: &str = "
    SELECT ruleset, position, title
    FROM ruleset_tests
    ORDER BY ruleset, position
";

/// Reads every expectation of every saved test.
///
/// Ordered by field so a listed test reads the same way twice, which is what
/// the round-trip comparison rests on.
const SELECT_TEST_VALUES: &str = "
    SELECT ruleset, position, field, expected
    FROM ruleset_test_values
    ORDER BY ruleset, position, field
";

/// The stored rulesets, read and written through one pool.
pub(crate) struct RulesetStore {
    pool: SqlitePool,
}

impl RulesetStore {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Returns every stored ruleset with its fields and saved tests, ordered
    /// by name.
    ///
    /// # Errors
    ///
    /// Returns a decode failure when a row names a kind this build does not
    /// know. Every stored value was one when it was written, so the row is
    /// corrupt rather than merely unexpected.
    pub(crate) async fn list(&self) -> Result<Vec<Ruleset>, sqlx::Error> {
        let mut rulesets =
            sqlx::query_as::<_, (String, String, Option<String>, bool, bool)>(SELECT_RULESETS)
                .fetch_all(&self.pool)
                .await?
                .into_iter()
                .map(|(id, name, based_on, template, enabled)| Ruleset {
                    id,
                    name,
                    enabled,
                    template,
                    based_on,
                    fields: Vec::new(),
                    tests: Vec::new(),
                })
                .collect::<Vec<_>>();

        for row in sqlx::query(SELECT_FIELDS).fetch_all(&self.pool).await? {
            let owner: String = row.try_get("ruleset")?;

            // A field outlives its ruleset row only if the foreign key falls,
            // so a miss here reports nothing.
            let Some(ruleset) = rulesets.iter_mut().find(|ruleset| ruleset.id == owner) else {
                continue;
            };

            ruleset.fields.push(field(&row)?);
        }

        for row in sqlx::query(SELECT_TESTS).fetch_all(&self.pool).await? {
            let owner: String = row.try_get("ruleset")?;

            let Some(ruleset) = rulesets.iter_mut().find(|ruleset| ruleset.id == owner) else {
                continue;
            };

            ruleset.tests.push(RulesetTest {
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
            let owner: String = row.try_get("ruleset")?;
            let position: i64 = row.try_get("position")?;

            let Some(test) = rulesets
                .iter_mut()
                .find(|ruleset| ruleset.id == owner)
                .and_then(|ruleset| ruleset.tests.get_mut(usize::try_from(position).ok()?))
            else {
                continue;
            };

            test.expected
                .insert(row.try_get("field")?, row.try_get("expected")?);
        }

        Ok(rulesets)
    }

    /// Writes `ruleset` with its fields and saved tests, replacing whatever
    /// was stored.
    ///
    /// Both lists are deleted and reinserted rather than updated in place,
    /// because a save drops a row as readily as it changes one. The whole
    /// write is one transaction, so a failure part way leaves the ruleset as
    /// it was rather than half rewritten.
    ///
    /// A ruleset already stored keeps its enabled state. Saving an edit is
    /// not a request to start or stop the ruleset.
    pub(crate) async fn upsert(&self, ruleset: &Ruleset) -> Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;

        sqlx::query(UPSERT)
            .bind(&ruleset.id)
            .bind(&ruleset.name)
            .bind(ruleset.based_on.as_deref())
            .bind(ruleset.template)
            .bind(ruleset.enabled)
            .execute(&mut *tx)
            .await?;

        sqlx::query("DELETE FROM ruleset_fields WHERE ruleset = ?1")
            .bind(&ruleset.id)
            .execute(&mut *tx)
            .await?;

        for (position, field) in ruleset.fields.iter().enumerate() {
            sqlx::query(
                "INSERT INTO ruleset_fields
                    (ruleset, position, name, kind, pattern, required, identity, tight)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )
            .bind(&ruleset.id)
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
        sqlx::query("DELETE FROM ruleset_tests WHERE ruleset = ?1")
            .bind(&ruleset.id)
            .execute(&mut *tx)
            .await?;

        for (position, test) in ruleset.tests.iter().enumerate() {
            let position = position as i64;

            sqlx::query("INSERT INTO ruleset_tests (ruleset, position, title) VALUES (?1, ?2, ?3)")
                .bind(&ruleset.id)
                .bind(position)
                .bind(&test.title)
                .execute(&mut *tx)
                .await?;

            for (field, expected) in &test.expected {
                sqlx::query(
                    "INSERT INTO ruleset_test_values (ruleset, position, field, expected)
                     VALUES (?1, ?2, ?3, ?4)",
                )
                .bind(&ruleset.id)
                .bind(position)
                .bind(field)
                .bind(expected)
                .execute(&mut *tx)
                .await?;
            }
        }

        tx.commit().await
    }

    /// Removes the ruleset `id` and its fields, and reports whether one was
    /// there.
    ///
    /// # Errors
    ///
    /// Returns a database error while another ruleset is based on this one.
    /// A ruleset whose template is gone resolves no field, so the delete
    /// fails instead.
    pub(crate) async fn remove(&self, id: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM rulesets WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() == 1)
    }

    /// Switches the ruleset `id` on or off, and reports whether one was there.
    pub(crate) async fn set_enabled(&self, id: &str, enabled: bool) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("UPDATE rulesets SET enabled = ?2 WHERE id = ?1")
            .bind(id)
            .bind(enabled)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() == 1)
    }
}

/// Rebuilds one field from its row.
///
/// The kind is stored as the same text the editor's form posts, so one
/// vocabulary serves the form and the table.
fn field(row: &sqlx::sqlite::SqliteRow) -> Result<Field, sqlx::Error> {
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

    use super::{Field, FieldKind, Ruleset, RulesetStore, RulesetTest};

    fn field(name: &str, pattern: Option<&str>) -> Field {
        Field {
            name: name.to_owned(),
            kind: FieldKind::Text,
            pattern: pattern.map(ToOwned::to_owned),
            required: true,
            tight: false,
            identity: false,
        }
    }

    fn ruleset(id: &str, template: bool, based_on: Option<&str>, fields: Vec<Field>) -> Ruleset {
        Ruleset {
            id: id.to_owned(),
            name: id.to_owned(),
            enabled: false,
            template,
            based_on: based_on.map(ToOwned::to_owned),
            fields,
            tests: Vec::new(),
        }
    }

    /// A template with two fields, whose order the position column keeps,
    /// and one saved test naming both of them.
    fn template() -> Ruleset {
        Ruleset {
            tests: vec![RulesetTest {
                title: "The.Hollow.Meridian.S04E06".to_owned(),
                expected: BTreeMap::from([
                    ("show".to_owned(), "the hollow meridian".to_owned()),
                    ("season".to_owned(), "4".to_owned()),
                ]),
            }],
            ..ruleset(
                "series",
                true,
                None,
                vec![field("show", Some(r"^(?<show>\w+)")), field("season", None)],
            )
        }
    }

    #[sqlx::test]
    async fn list_of_a_fresh_database_is_empty(pool: SqlitePool) {
        assert_eq!(
            RulesetStore::new(pool).list().await.expect("list"),
            Vec::new()
        );
    }

    #[sqlx::test]
    async fn upsert_then_list_round_trips_each_ruleset(pool: SqlitePool) {
        let store = RulesetStore::new(pool);
        store.upsert(&template()).await.expect("the base");

        // One field overrides the helper's flag, so both values cross the
        // table under the comparison below.
        let tight_show = || Field {
            tight: true,
            ..field("show", Some("^Ashfall"))
        };

        store
            .upsert(&ruleset(
                "archive",
                false,
                Some("series"),
                vec![tight_show(), field("year", Some(r"\.\d{4}"))],
            ))
            .await
            .expect("the ruleset on it");

        assert_eq!(
            store.list().await.expect("list"),
            vec![
                ruleset(
                    "archive",
                    false,
                    Some("series"),
                    vec![tight_show(), field("year", Some(r"\.\d{4}"))]
                ),
                template(),
            ],
            "ordered by name, each with its fields in position order"
        );
    }

    #[sqlx::test]
    async fn a_second_upsert_replaces_the_fields_and_keeps_enabled(pool: SqlitePool) {
        let store = RulesetStore::new(pool);
        store.upsert(&template()).await.expect("the template");
        store.set_enabled("series", true).await.expect("enable");

        let mut edited = ruleset("series", true, None, vec![field("title", Some("^."))]);
        store.upsert(&edited).await.expect("the edit");
        edited.enabled = true;

        assert_eq!(
            store.list().await.expect("list"),
            vec![edited],
            "a dropped field and a dropped test both go, and the switch is not part of the edit"
        );
    }

    #[sqlx::test]
    async fn remove_reports_the_row_and_cascades_the_fields(pool: SqlitePool) {
        let store = RulesetStore::new(pool.clone());
        store.upsert(&template()).await.expect("the base");

        assert!(
            store.remove("series").await.expect("remove"),
            "a stored row"
        );
        assert!(
            !store.remove("series").await.expect("remove again"),
            "nothing left to remove"
        );
        assert_eq!(
            store.list().await.expect("list"),
            Vec::new(),
            "the fields go with the ruleset that owned them"
        );

        // The values hang off the tests, which hang off the ruleset, so this
        // is the far end of the chain and the one a single delete has to
        // reach.
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM ruleset_test_values")
                .fetch_one(&pool)
                .await
                .expect("count"),
            0,
            "and the expectations go with the tests that named them"
        );
    }

    #[sqlx::test]
    async fn removing_a_template_a_ruleset_is_based_on_fails(pool: SqlitePool) {
        let store = RulesetStore::new(pool);
        store.upsert(&template()).await.expect("the base");
        store
            .upsert(&ruleset("archive", false, Some("series"), Vec::new()))
            .await
            .expect("the ruleset on it");

        assert!(
            store.remove("series").await.is_err(),
            "a ruleset whose template is gone parses no title"
        );
    }

    #[sqlx::test]
    async fn set_enabled_flips_and_reports(pool: SqlitePool) {
        let store = RulesetStore::new(pool);
        store.upsert(&template()).await.expect("the base");

        assert!(
            store.set_enabled("series", true).await.expect("enable"),
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
