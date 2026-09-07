//! The preference lists a reader states, kept between restarts.
//!
//! One row per value, ordered by position within a field. A write replaces
//! one field's whole list, because a reader reorders a list as readily as
//! they add to it.

use std::collections::BTreeMap;

use sqlx::SqlitePool;

use super::Preferences;

/// Reads every list, grouped by field and in preference order.
const SELECT: &str = "SELECT field, position, value FROM preferences ORDER BY field, position";

/// Returns every stated list.
///
/// The whole set comes back at once, because ranking a listing tests every
/// row against the same lists. A query per field costs one round trip per
/// field the parser reads.
pub(crate) async fn all(pool: &SqlitePool) -> Result<Preferences, sqlx::Error> {
    let rows: Vec<(String, i64, String)> = sqlx::query_as(SELECT).fetch_all(pool).await?;

    let mut lists: BTreeMap<String, Vec<String>> = BTreeMap::new();

    // The rows arrive in position order, so pushing in turn rebuilds each
    // list as it was written.
    for (field, _, value) in rows {
        lists.entry(field).or_default().push(value);
    }

    Ok(Preferences::new(lists))
}

/// Replaces the list stated for `field`.
///
/// An empty `values` leaves the field with no list, which is how a reader
/// takes back a statement rather than a way of stating nothing.
///
/// The whole write is one transaction, so a failure part way leaves the
/// previous list rather than half of the new one.
pub(crate) async fn replace(
    pool: &SqlitePool,
    field: &str,
    values: &[String],
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM preferences WHERE field = ?1")
        .bind(field)
        .execute(&mut *tx)
        .await?;

    for (position, value) in values.iter().enumerate() {
        sqlx::query("INSERT INTO preferences (field, position, value) VALUES (?1, ?2, ?3)")
            .bind(field)
            .bind(position as i64)
            .bind(value)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use sqlx::SqlitePool;

    use super::{Preferences, all, replace};

    fn list(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    fn stated(lists: &[(&str, &[&str])]) -> Preferences {
        Preferences::new(
            lists
                .iter()
                .map(|(field, values)| ((*field).to_owned(), list(values)))
                .collect::<BTreeMap<_, _>>(),
        )
    }

    #[sqlx::test]
    async fn a_list_reads_back_in_the_order_it_was_written(pool: SqlitePool) {
        replace(&pool, "resolution", &list(&["2160p", "1080p", "720p"]))
            .await
            .expect("write");

        assert_eq!(
            all(&pool).await.expect("read"),
            stated(&[("resolution", &["2160p", "1080p", "720p"])]),
            "position orders the values, not the text"
        );
    }

    #[sqlx::test]
    async fn an_empty_write_drops_the_field(pool: SqlitePool) {
        replace(&pool, "resolution", &list(&["2160p"]))
            .await
            .expect("write");
        replace(&pool, "resolution", &[]).await.expect("take back");

        assert_eq!(
            all(&pool).await.expect("read"),
            Preferences::default(),
            "a field with no list is absent rather than empty"
        );
    }

    #[sqlx::test]
    async fn one_field_write_leaves_another_alone(pool: SqlitePool) {
        replace(&pool, "resolution", &list(&["2160p", "1080p"]))
            .await
            .expect("write");
        replace(&pool, "publisher", &list(&["PublicWave", "OtherGroup"]))
            .await
            .expect("write");
        replace(&pool, "resolution", &list(&["1080p"]))
            .await
            .expect("rewrite");

        assert_eq!(
            all(&pool).await.expect("read"),
            stated(&[
                ("publisher", &["PublicWave", "OtherGroup"]),
                ("resolution", &["1080p"]),
            ]),
            "the delete names one field, so the other keeps its list"
        );
    }
}
