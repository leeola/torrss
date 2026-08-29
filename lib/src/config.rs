//! What a file declares about the feeds and the torrent client.
//!
//! The admin page is one way to register a feed; this is the other. A file
//! states what must exist, and every file named on the command line is
//! applied at startup in the order it was given.
//!
//! A later file wins per field, which is what the partial declaration types
//! are for: a file in the Nix store names the client's address and account,
//! and a secret file outside it names only the password. Neither erases what
//! the other set.
//!
//! Nothing is ever removed. A feed registered on the admin page stays, and a
//! declaration dropped from a file leaves its row in place. A file says what
//! must exist rather than what may.

use serde::Deserialize;
use sqlx::SqlitePool;
use url::Url;

use crate::feed::FeedAuth;
use crate::feed::store::FeedStore;
use crate::torrent::ClientDeclaration;
use crate::torrent::store::ClientStore;

/// One configuration file.
///
/// `deny_unknown_fields` turns a typo into a startup error. A key silently
/// ignored is the worst failure this file has: everything reads as fine and
/// nothing was applied.
#[derive(Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigFile {
    #[serde(default)]
    pub feeds: Vec<FeedDeclaration>,

    pub qbit: Option<ClientDeclaration>,
}

/// One feed a file declares.
#[derive(Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedDeclaration {
    pub name: String,
    pub url: Url,
    pub auth: Option<FeedAuth>,
}

impl ConfigFile {
    /// Reads one file's text.
    ///
    /// # Errors
    ///
    /// Returns the parse error, which names the line and the key at fault.
    pub fn parse(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }

    /// Writes every declaration into the stores.
    ///
    /// The feeds go in the order the file lists them, then the client. Each
    /// upsert keeps what the declaration does not name, so applying one file
    /// after another merges them.
    ///
    /// # Errors
    ///
    /// Returns the store's error when a write fails. Whatever was applied
    /// before it stays, because a half-applied file is still closer to what
    /// the file asks for than none of it.
    pub async fn apply(&self, pool: &SqlitePool) -> Result<(), sqlx::Error> {
        let feeds = FeedStore::new(pool.clone());

        for feed in &self.feeds {
            feeds
                .upsert(&feed.name, &feed.url, feed.auth.as_ref())
                .await?;
        }

        if let Some(qbit) = &self.qbit {
            ClientStore::new(pool.clone()).upsert(qbit).await?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use sqlx::SqlitePool;
    use url::Url;

    use super::{ConfigFile, FeedDeclaration};
    use crate::feed::store::FeedStore;
    use crate::feed::{BasicAuth, FeedAuth};
    use crate::torrent::store::ClientStore;
    use crate::torrent::{ClientDeclaration, ClientSettings};

    const BOTH_FEEDS: &str = r#"
        [[feeds]]
        name = "Public"
        url = "https://public.invalid/rss"

        [[feeds]]
        name = "Private"
        url = "https://private.invalid/rss?passkey=abc"

        [feeds.auth.basic]
        username = "reader"
        password = "hunter2"

        [feeds.auth.headers]
        Cookie = "uid=1; pass=abc"

        [qbit]
        password = "from-the-secret-file"
    "#;

    fn url(raw: &str) -> Url {
        Url::parse(raw).expect("the test URL parses")
    }

    #[test]
    fn empty_text_is_default() {
        assert_eq!(
            ConfigFile::parse("").expect("an empty file parses"),
            ConfigFile::default(),
            "a file declaring nothing is a file that changes nothing"
        );
    }

    #[test]
    fn feeds_and_qbit_parse() {
        assert_eq!(
            ConfigFile::parse(BOTH_FEEDS).expect("parse"),
            ConfigFile {
                feeds: vec![
                    FeedDeclaration {
                        name: "Public".to_owned(),
                        url: url("https://public.invalid/rss"),
                        auth: None,
                    },
                    FeedDeclaration {
                        name: "Private".to_owned(),
                        url: url("https://private.invalid/rss?passkey=abc"),
                        auth: Some(FeedAuth {
                            basic: Some(BasicAuth {
                                username: "reader".to_owned(),
                                password: "hunter2".to_owned(),
                            }),
                            headers: BTreeMap::from([(
                                "Cookie".to_owned(),
                                "uid=1; pass=abc".to_owned(),
                            )]),
                        }),
                    },
                ],
                qbit: Some(ClientDeclaration {
                    url: None,
                    username: None,
                    password: Some("from-the-secret-file".to_owned()),
                }),
            }
        );
    }

    #[test]
    fn unknown_key_is_error() {
        assert!(
            ConfigFile::parse("[[feeds]]\nname = \"A\"\nurl = \"https://a.invalid/\"\nfeed = 1\n")
                .is_err(),
            "a typo is a startup error, not a key that is quietly dropped"
        );
    }

    #[test]
    fn invalid_url_is_error() {
        assert!(
            ConfigFile::parse("[[feeds]]\nname = \"A\"\nurl = \"not a url\"\n").is_err(),
            "a URL that never parses fails when the file is read, not on the first fetch"
        );
    }

    #[test]
    fn debug_keeps_no_password() {
        let rendered = format!("{:?}", ConfigFile::parse(BOTH_FEEDS).expect("parse"));

        assert!(
            !rendered.contains("hunter2") && !rendered.contains("from-the-secret-file"),
            "a parsed file is what a startup failure formats: {rendered}"
        );
    }

    #[sqlx::test]
    async fn apply_twice_leaves_one_row_per_url(pool: SqlitePool) {
        let config = ConfigFile::parse(BOTH_FEEDS).expect("parse");
        config.apply(&pool).await.expect("first apply");
        config.apply(&pool).await.expect("second apply");

        let feeds = FeedStore::new(pool)
            .list()
            .await
            .expect("list")
            .into_iter()
            .map(|feed| (feed.id, feed.name))
            .collect::<Vec<_>>();

        assert_eq!(
            feeds,
            vec![(1, "Public".to_owned()), (2, "Private".to_owned())],
            "applying one declaration twice changes nothing"
        );
    }

    #[sqlx::test]
    async fn later_file_wins_per_field(pool: SqlitePool) {
        ConfigFile::parse("[qbit]\nurl = \"http://qbit.invalid:9090/\"\nusername = \"reader\"\n")
            .expect("the store-side file")
            .apply(&pool)
            .await
            .expect("apply");

        ConfigFile::parse("[qbit]\npassword = \"from-the-secret-file\"\n")
            .expect("the secret file")
            .apply(&pool)
            .await
            .expect("apply");

        assert_eq!(
            ClientStore::new(pool).load().await.expect("load"),
            ClientSettings {
                url: url("http://qbit.invalid:9090/"),
                username: "reader".to_owned(),
                password: "from-the-secret-file".to_owned(),
            },
            "the second file adds the password without erasing the address"
        );
    }
}
