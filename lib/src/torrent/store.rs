//! The torrent client connection, kept between restarts.
//!
//! One row, so the table holds the one client the services carry. A
//! declaration writes the fields it names and leaves the rest, which is what
//! lets one file state the address and another state only the password.

use sqlx::SqlitePool;
use url::Url;

use super::{ClientDeclaration, ClientSettings};

/// Writes the fields a declaration names, and keeps the rest.
///
/// The same `coalesce` shape the feeds table uses. Applying one file after
/// another merges them rather than letting the last one erase the rest, which
/// is what makes a secret file separate from a public one work at all.
const UPSERT: &str = "
    INSERT INTO torrent_client (id, url, username, password)
    VALUES (1, ?1, ?2, ?3)
    ON CONFLICT (id) DO UPDATE SET
        url = coalesce(excluded.url, url),
        username = coalesce(excluded.username, username),
        password = coalesce(excluded.password, password)
";

const SELECT: &str = "SELECT url, username, password FROM torrent_client WHERE id = 1";

/// The stored connection, read and written through one pool.
pub struct ClientStore {
    pool: SqlitePool,
}

impl ClientStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Applies `declaration`, leaving every field it does not name.
    ///
    /// # Errors
    ///
    /// Returns the store's error when the write fails.
    pub async fn upsert(&self, declaration: &ClientDeclaration) -> Result<(), sqlx::Error> {
        sqlx::query(UPSERT)
            .bind(declaration.url.as_ref().map(Url::as_str))
            .bind(declaration.username.as_deref())
            .bind(declaration.password.as_deref())
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Returns the stored connection, with anything unset left at its default.
    ///
    /// A database with no row answers with the whole default, so a first run
    /// reaches a local qBittorrent without being told where one is.
    ///
    /// # Errors
    ///
    /// Returns the store's error when the read fails, or when the stored URL
    /// no longer parses. It parsed when written, so that means the row is
    /// corrupt.
    pub async fn load(&self) -> Result<ClientSettings, sqlx::Error> {
        let Some((url, username, password)) =
            sqlx::query_as::<_, (Option<String>, Option<String>, Option<String>)>(SELECT)
                .fetch_optional(&self.pool)
                .await?
        else {
            return Ok(ClientSettings::default());
        };

        let mut settings = ClientSettings::default();

        if let Some(url) = url {
            settings.url = Url::parse(&url).map_err(sqlx::Error::decode)?;
        }

        if let Some(username) = username {
            settings.username = username;
        }

        if let Some(password) = password {
            settings.password = password;
        }

        Ok(settings)
    }
}

#[cfg(test)]
mod tests {
    use sqlx::SqlitePool;
    use url::Url;

    use super::{ClientDeclaration, ClientSettings, ClientStore};

    fn url(raw: &str) -> Url {
        Url::parse(raw).expect("the test URL parses")
    }

    fn full() -> ClientDeclaration {
        ClientDeclaration {
            url: Some(url("http://qbit.invalid:9090/")),
            username: Some("reader".to_owned()),
            password: Some("hunter2".to_owned()),
        }
    }

    #[sqlx::test]
    async fn load_without_row_is_default(pool: SqlitePool) {
        let client = ClientStore::new(pool);

        assert_eq!(
            client.load().await.expect("load"),
            ClientSettings::default(),
            "a first run reaches a local client without being told where one is"
        );
    }

    #[sqlx::test]
    async fn upsert_then_load_round_trips(pool: SqlitePool) {
        let client = ClientStore::new(pool);
        client.upsert(&full()).await.expect("upsert");

        assert_eq!(
            client.load().await.expect("load"),
            ClientSettings {
                url: url("http://qbit.invalid:9090/"),
                username: "reader".to_owned(),
                password: "hunter2".to_owned(),
            }
        );
    }

    #[sqlx::test]
    async fn partial_upsert_keeps_other_fields(pool: SqlitePool) {
        let client = ClientStore::new(pool);
        client.upsert(&full()).await.expect("the public file");

        client
            .upsert(&ClientDeclaration {
                password: Some("from-the-secret-file".to_owned()),
                ..ClientDeclaration::default()
            })
            .await
            .expect("the secret file");

        assert_eq!(
            client.load().await.expect("load"),
            ClientSettings {
                url: url("http://qbit.invalid:9090/"),
                username: "reader".to_owned(),
                password: "from-the-secret-file".to_owned(),
            },
            "a file that names only the password leaves the address alone"
        );
    }

    #[sqlx::test]
    async fn upsert_twice_leaves_one_row(pool: SqlitePool) {
        let client = ClientStore::new(pool.clone());
        client.upsert(&full()).await.expect("first");
        client.upsert(&full()).await.expect("second");

        let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM torrent_client")
            .fetch_one(&pool)
            .await
            .expect("count");

        assert_eq!(
            rows, 1,
            "the services carry one client, so the table holds one"
        );
    }

    #[test]
    fn debug_keeps_no_password() {
        let declared = format!("{:?}", full());
        let settings = format!(
            "{:?}",
            ClientSettings {
                url: url("http://qbit.invalid:9090/"),
                username: "reader".to_owned(),
                password: "hunter2".to_owned(),
            }
        );

        assert!(
            !declared.contains("hunter2") && !settings.contains("hunter2"),
            "a formatted connection reaches the log: {declared} {settings}"
        );
        assert!(
            declared.contains("reader") && settings.contains("qbit.invalid"),
            "the account and the address say which client is meant: {declared} {settings}"
        );
    }
}
