//! Turning one announced release into a torrent the client holds.
//!
//! This is where the feed, the downloader, and the torrent client meet. A
//! stored item goes in; the client ends up holding the release, and the
//! grabs table ends up holding the outcome.
//!
//! Every attempt is recorded, not only the ones that work. A failed grab is
//! the case a reader most needs explained, so recording only successes would
//! hide exactly what the page has to show.

// FIXME: This module belongs to the crate rather than to its API. It is
// public only because the handler that calls it does not exist yet, and a
// `pub(crate)` item no caller reaches reads as dead code. Narrow it
// alongside `store::grabs`, once the handler lands.

use snafu::{ResultExt, Snafu};
use sqlx::SqlitePool;

use crate::clock::Clock;
use crate::download::{DownloadError, Downloader};
use crate::store::StoredItem;
use crate::store::grabs;
use crate::torrent::{AddTorrent, TorrentClient, TorrentError, TorrentSource};

/// Why a grab did not reach the torrent client.
///
/// Each variant displays its source alone. The variant names the stage that
/// failed, and a reader of the message wants the failure rather than the
/// stage.
#[derive(Debug, Snafu)]
pub enum GrabError {
    /// The torrent file never arrived, so there was nothing to hand over.
    #[snafu(display("{source}"))]
    Download { source: DownloadError },

    /// The client refused the torrent, or never answered.
    #[snafu(display("{source}"))]
    Client { source: TorrentError },

    /// The attempt itself went through, and recording it did not.
    #[snafu(display("{source}"))]
    Store { source: sqlx::Error },
}

/// Grabs `item` and records how it went.
///
/// A magnet link goes to the client untouched. Any other link is downloaded
/// first, because it carries the passkey and cookies that fetched the feed
/// and the client cannot present either.
///
/// The recorded outcome survives a failure, so the page shows a grab that
/// did not work. When both the grab and the recording fail, the grab's error
/// is the one returned: that is the failure the caller asked about.
pub async fn grab(
    pool: &SqlitePool,
    downloader: &dyn Downloader,
    client: &dyn TorrentClient,
    clock: &dyn Clock,
    item: &StoredItem,
) -> Result<(), GrabError> {
    let submitted = submit(downloader, client, item).await;

    let recorded = grabs::record(
        pool,
        item.id,
        clock.now(),
        submitted.as_ref().err().map(ToString::to_string).as_deref(),
    )
    .await
    .context(StoreSnafu);

    submitted.and(recorded)
}

/// Hands the release to the torrent client.
async fn submit(
    downloader: &dyn Downloader,
    client: &dyn TorrentClient,
    item: &StoredItem,
) -> Result<(), GrabError> {
    let source = source(downloader, item).await?;

    client
        .add(&AddTorrent::new(source))
        .await
        .context(ClientSnafu)
}

/// Returns what the client should be given for `item`.
///
/// The filename only has to be non-empty. qBittorrent reads the release name
/// out of the torrent's own metadata, so this one is never what the user
/// sees.
async fn source(
    downloader: &dyn Downloader,
    item: &StoredItem,
) -> Result<TorrentSource, GrabError> {
    if item.item.link.scheme() == "magnet" {
        return Ok(TorrentSource::Magnet(item.item.link.clone()));
    }

    let data = downloader
        .download(&item.item.link)
        .await
        .context(DownloadSnafu)?;

    Ok(TorrentSource::File {
        filename: format!("{}.torrent", item.item.title),
        data,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::{TimeZone, Utc};
    use sqlx::SqlitePool;
    use url::Url;

    use super::grab;
    use crate::clock::Clock;
    use crate::download::DownloadError;
    use crate::feed::{FeedItem, fake};
    use crate::services::Services;
    use crate::store;
    use crate::store::grabs::{self, Grab};
    use crate::torrent::{AddTorrent, TorrentError, TorrentSource};

    const FEED: &str = "https://tracker.invalid/rss";
    const TITLE: &str = "Show.S01E01";
    const TORRENT_URL: &str = "https://fake.invalid/Show.S01E01.torrent";
    const MAGNET: &str = "magnet:?xt=urn:btih:0123456789abcdef&dn=Show.S01E01";
    const BYTES: &[u8] = b"d8:announce4:teste";

    fn url(raw: &str) -> Url {
        Url::parse(raw).expect("the test URL parses")
    }

    /// Stores `item` and hands back the row, which is what a grab takes.
    async fn stored(pool: &SqlitePool, item: FeedItem) -> store::StoredItem {
        let seen = Utc
            .with_ymd_and_hms(2025, 3, 1, 12, 0, 0)
            .single()
            .expect("the test date is unambiguous");

        store::ingest(pool, &url(FEED), seen, &[item])
            .await
            .expect("ingest");

        store::item(pool, 1)
            .await
            .expect("item")
            .expect("the row was just stored")
    }

    #[sqlx::test]
    async fn magnet_link_adds_without_download(pool: SqlitePool) {
        let (services, fakes) = Services::fake(pool);
        let item = stored(&services.db, fake::item(TITLE).magnet(MAGNET)).await;

        grab(
            &services.db,
            services.downloads.as_ref(),
            services.torrents.as_ref(),
            services.clock.as_ref(),
            &item,
        )
        .await
        .expect("grab");

        assert_eq!(
            fakes.downloads.downloaded(),
            Vec::new(),
            "a magnet needs nothing fetched"
        );
        assert_eq!(
            fakes.torrents.added(),
            vec![AddTorrent::new(TorrentSource::Magnet(url(MAGNET)))],
            "the link reaches the client untouched"
        );
    }

    #[sqlx::test]
    async fn torrent_url_downloads_then_uploads_bytes(pool: SqlitePool) {
        let (services, fakes) = Services::fake(pool);
        let item = stored(&services.db, fake::item(TITLE)).await;
        fakes.downloads.file(TORRENT_URL, BYTES);

        grab(
            &services.db,
            services.downloads.as_ref(),
            services.torrents.as_ref(),
            services.clock.as_ref(),
            &item,
        )
        .await
        .expect("grab");

        assert_eq!(
            fakes.downloads.downloaded(),
            vec![url(TORRENT_URL)],
            "the link is fetched once"
        );
        assert_eq!(
            fakes.torrents.added(),
            vec![AddTorrent::new(TorrentSource::File {
                filename: format!("{TITLE}.torrent"),
                data: BYTES.to_vec(),
            })],
            "the bytes go to the client, not the URL"
        );
    }

    #[sqlx::test]
    async fn download_failure_records_error(pool: SqlitePool) {
        let (services, fakes) = Services::fake(pool);
        let item = stored(&services.db, fake::item(TITLE)).await;
        fakes
            .downloads
            .failing(TORRENT_URL, DownloadError::Status { code: 403 });

        assert!(
            grab(
                &services.db,
                services.downloads.as_ref(),
                services.torrents.as_ref(),
                services.clock.as_ref(),
                &item,
            )
            .await
            .is_err()
        );

        assert_eq!(
            fakes.torrents.added(),
            Vec::new(),
            "nothing reaches the client when the file never arrives"
        );
        assert_eq!(
            grabs::all(&services.db).await.expect("grabs"),
            HashMap::from([(
                item.id,
                Grab {
                    item_id: item.id,
                    at: fakes.clock.now(),
                    error: Some("the download answered with status 403".to_owned()),
                }
            )]),
            "a failed attempt is recorded, with the reason"
        );
    }

    #[sqlx::test]
    async fn client_rejection_records_error(pool: SqlitePool) {
        let (services, fakes) = Services::fake(pool);
        let item = stored(&services.db, fake::item(TITLE)).await;
        fakes.downloads.file(TORRENT_URL, BYTES);
        fakes.torrents.fail_next(TorrentError::Rejected {
            reason: "duplicate".to_owned(),
        });

        assert!(
            grab(
                &services.db,
                services.downloads.as_ref(),
                services.torrents.as_ref(),
                services.clock.as_ref(),
                &item,
            )
            .await
            .is_err()
        );

        assert_eq!(
            grabs::all(&services.db).await.expect("grabs"),
            HashMap::from([(
                item.id,
                Grab {
                    item_id: item.id,
                    at: fakes.clock.now(),
                    error: Some("the torrent client rejected the request: duplicate".to_owned()),
                }
            )]),
            "the client's own words are what the page shows"
        );
    }

    #[sqlx::test]
    async fn success_records_a_grab_with_no_error(pool: SqlitePool) {
        let (services, fakes) = Services::fake(pool);
        let item = stored(&services.db, fake::item(TITLE).magnet(MAGNET)).await;

        grab(
            &services.db,
            services.downloads.as_ref(),
            services.torrents.as_ref(),
            services.clock.as_ref(),
            &item,
        )
        .await
        .expect("grab");

        assert_eq!(
            grabs::all(&services.db).await.expect("grabs"),
            HashMap::from([(
                item.id,
                Grab {
                    item_id: item.id,
                    at: fakes.clock.now(),
                    error: None,
                }
            )])
        );
    }
}
