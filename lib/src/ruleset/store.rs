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

use sqlx::{Row, SqlitePool};

use super::{Field, FieldKind, Part, Ruleset};

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
    SELECT ruleset, name, part, kind, pattern, required, identity
    FROM ruleset_fields
    ORDER BY ruleset, position
";

/// The stored rulesets, read and written through one pool.
pub(crate) struct RulesetStore {
    pool: SqlitePool,
}

impl RulesetStore {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Returns every stored ruleset with its fields, ordered by name.
    ///
    /// # Errors
    ///
    /// Returns a decode failure when a row names a part or a kind this build
    /// does not know. Every stored value was one of those when it was
    /// written, so the row is corrupt rather than merely unexpected.
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

        Ok(rulesets)
    }

    /// Writes `ruleset` and its fields, replacing whatever was stored.
    ///
    /// The fields are deleted and reinserted rather than updated in place,
    /// because a save drops a field as readily as it changes one. The whole
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
                    (ruleset, position, name, part, kind, pattern, required, identity)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )
            .bind(&ruleset.id)
            .bind(position as i64)
            .bind(&field.name)
            .bind(field.part.slug())
            .bind(field.kind.label())
            .bind(field.pattern.as_deref())
            .bind(field.required)
            .bind(field.identity)
            .execute(&mut *tx)
            .await?;
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
/// The part and the kind are stored as the same text the editor's form posts,
/// so one vocabulary serves the URL, the form, and the table.
fn field(row: &sqlx::sqlite::SqliteRow) -> Result<Field, sqlx::Error> {
    let part: String = row.try_get("part")?;
    let kind: String = row.try_get("kind")?;

    Ok(Field {
        name: row.try_get("name")?,
        part: Part::from_slug(&part)
            .ok_or_else(|| sqlx::Error::decode(format!("unknown part {part}")))?,
        kind: FieldKind::from_label(&kind)
            .ok_or_else(|| sqlx::Error::decode(format!("unknown field kind {kind}")))?,
        pattern: row.try_get("pattern")?,
        required: row.try_get("required")?,
        identity: row.try_get("identity")?,
    })
}

#[cfg(test)]
mod tests {
    use sqlx::SqlitePool;

    use super::{Field, FieldKind, Part, Ruleset, RulesetStore};

    fn field(name: &str, part: Part, pattern: Option<&str>) -> Field {
        Field {
            name: name.to_owned(),
            part,
            kind: FieldKind::Text,
            pattern: pattern.map(ToOwned::to_owned),
            required: true,
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
        }
    }

    /// A template with two fields, whose order the position column keeps.
    fn template() -> Ruleset {
        ruleset(
            "series",
            true,
            None,
            vec![
                field("show", Part::Show, Some(r"^(?<show>\w+)")),
                field("season", Part::Season, None),
            ],
        )
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
        store
            .upsert(&ruleset(
                "archive",
                false,
                Some("series"),
                vec![field("show", Part::Show, Some("^Ashfall"))],
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
                    vec![field("show", Part::Show, Some("^Ashfall"))]
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

        let mut edited = ruleset(
            "series",
            true,
            None,
            vec![field("title", Part::Movie, Some("^."))],
        );
        store.upsert(&edited).await.expect("the edit");
        edited.enabled = true;

        assert_eq!(
            store.list().await.expect("list"),
            vec![edited],
            "a dropped field goes, and the switch is not part of the edit"
        );
    }

    #[sqlx::test]
    async fn remove_reports_the_row_and_cascades_the_fields(pool: SqlitePool) {
        let store = RulesetStore::new(pool);
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
