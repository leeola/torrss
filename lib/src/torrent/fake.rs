//! A [`TorrentClient`] that keeps its torrents in memory.
//!
//! A test drives the whole grab path against this and then asserts what the
//! client was asked to add, without a qBittorrent to run or clean up after.
//!
//! The simulation goes only as far as the application observes. A torrent
//! never progresses on its own, so a test that needs a finished download
//! says so with [`FakeTorrents::set_state`].

use std::sync::{Mutex, MutexGuard};

use async_trait::async_trait;

use super::{
    AddTorrent, ClientInfo, Torrent, TorrentClient, TorrentError, TorrentId, TorrentSource,
    TorrentState,
};

/// An in-memory torrent client.
///
/// Every method takes `&self`, because a test holds this through an
/// `Arc<FakeTorrents>` and scripts it after the services it lives in are
/// built.
#[derive(Debug, Default)]
pub struct FakeTorrents {
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    added: Vec<AddTorrent>,
    torrents: Vec<Torrent>,
    fail_next: Option<TorrentError>,
    reject_duplicates: bool,

    /// Counts every id ever issued, so a removal never frees one for reuse.
    ///
    /// Numbering from the current length instead reissues a live id after a
    /// removal, which leaves `remove` and `set_state` acting on the wrong
    /// torrent.
    issued: u32,
}

impl FakeTorrents {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns every add request, in the order it arrived.
    pub fn added(&self) -> Vec<AddTorrent> {
        self.lock().added.clone()
    }

    /// Returns the text identifying each add request.
    ///
    /// A magnet or URL source renders as the URL, and a file source as its
    /// filename. Every request contributes one entry, so a count here
    /// matches a count of [`Self::added`].
    pub fn added_urls(&self) -> Vec<String> {
        self.lock()
            .added
            .iter()
            .map(|request| match &request.source {
                TorrentSource::Magnet(url) | TorrentSource::Url(url) => url.to_string(),
                TorrentSource::File { filename, .. } => filename.clone(),
            })
            .collect()
    }

    /// Returns the id of every torrent the client holds, in order.
    pub fn ids(&self) -> Vec<TorrentId> {
        self.lock()
            .torrents
            .iter()
            .map(|torrent| torrent.id.clone())
            .collect()
    }

    /// Adds a torrent named `name` that is already seeding.
    ///
    /// This records no add request, so it states what the client already
    /// held rather than what the test asked for.
    pub fn seed(&self, name: &str) -> TorrentId {
        let mut inner = self.lock();
        let id = inner.next_id();

        inner.torrents.push(Torrent {
            id: id.clone(),
            name: name.to_owned(),
            state: TorrentState::Seeding,
            size: 0,
            progress: 1.0,
        });

        id
    }

    /// Moves the torrent `id` into `state`.
    ///
    /// # Panics
    ///
    /// Panics when no torrent carries `id`. A test holds an id from
    /// [`Self::seed`] or [`Self::ids`], so an unknown one is a test bug.
    pub fn set_state(&self, id: &TorrentId, state: TorrentState) {
        let mut inner = self.lock();

        match inner.torrents.iter_mut().find(|torrent| &torrent.id == id) {
            Some(torrent) => torrent.state = state,
            None => panic!("no fake torrent carries the id {}", id.0),
        }
    }

    /// Fails the next call with `error`, whichever call that turns out to be.
    ///
    /// The error applies once. The call after it succeeds, which is how a
    /// test covers a retry.
    pub fn fail_next(&self, error: TorrentError) {
        self.lock().fail_next = Some(error);
    }

    /// Rejects an add whose source was added before.
    pub fn reject_duplicates(&self) {
        self.lock().reject_duplicates = true;
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // Nothing panics while the guard is held, so the lock never poisons.
        self.inner
            .lock()
            .expect("the fake torrent client lock is never poisoned")
    }
}

impl Inner {
    fn next_id(&mut self) -> TorrentId {
        self.issued += 1;
        TorrentId(format!("t{}", self.issued))
    }

    fn take_failure(&mut self) -> Result<(), TorrentError> {
        match self.fail_next.take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

#[async_trait]
impl TorrentClient for FakeTorrents {
    async fn add(&self, request: &AddTorrent) -> Result<(), TorrentError> {
        let mut inner = self.lock();
        inner.take_failure()?;

        if inner.reject_duplicates
            && inner
                .added
                .iter()
                .any(|earlier| earlier.source == request.source)
        {
            return Err(TorrentError::Rejected {
                reason: "duplicate".to_owned(),
            });
        }

        inner.added.push(request.clone());

        let id = inner.next_id();
        inner.torrents.push(Torrent {
            id,
            name: name(&request.source),
            state: TorrentState::Queued,
            size: 0,
            progress: 0.0,
        });

        Ok(())
    }

    async fn list(&self) -> Result<Vec<Torrent>, TorrentError> {
        let mut inner = self.lock();
        inner.take_failure()?;

        Ok(inner.torrents.clone())
    }

    async fn remove(&self, id: &TorrentId, _delete_files: bool) -> Result<(), TorrentError> {
        let mut inner = self.lock();
        inner.take_failure()?;

        let before = inner.torrents.len();
        inner.torrents.retain(|torrent| &torrent.id != id);

        if inner.torrents.len() == before {
            return Err(TorrentError::NotFound);
        }

        Ok(())
    }

    async fn check(&self) -> Result<ClientInfo, TorrentError> {
        self.lock().take_failure()?;

        Ok(ClientInfo {
            version: "fake".to_owned(),
        })
    }
}

/// Returns the name qBittorrent reports for a torrent added from `source`.
///
/// A magnet carries the release name in its `dn` value, and a torrent file
/// carries it in the filename. A plain URL carries it nowhere until the
/// client downloads the file, so the URL text stands in.
fn name(source: &TorrentSource) -> String {
    match source {
        TorrentSource::Magnet(url) => url
            .query_pairs()
            .find(|(key, _)| key == "dn")
            .map(|(_, value)| value.into_owned())
            .unwrap_or_else(|| url.to_string()),
        TorrentSource::Url(url) => url.to_string(),
        TorrentSource::File { filename, .. } => filename
            .strip_suffix(".torrent")
            .unwrap_or(filename)
            .to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use url::Url;

    use super::{
        AddTorrent, ClientInfo, FakeTorrents, Torrent, TorrentClient, TorrentError, TorrentId,
        TorrentSource, TorrentState,
    };

    fn magnet(name: &str) -> AddTorrent {
        let url = Url::parse(&format!("magnet:?xt=urn:btih:0123456789abcdef&dn={name}"))
            .expect("the test magnet link parses");

        AddTorrent::new(TorrentSource::Magnet(url))
    }

    fn queued(id: &str, name: &str) -> Torrent {
        Torrent {
            id: TorrentId(id.to_owned()),
            name: name.to_owned(),
            state: TorrentState::Queued,
            size: 0,
            progress: 0.0,
        }
    }

    #[tokio::test]
    async fn add_assigns_sequential_ids() {
        let torrents = FakeTorrents::new();
        torrents.add(&magnet("First.Release")).await.expect("add");
        torrents.add(&magnet("Second.Release")).await.expect("add");

        assert_eq!(
            torrents.list().await,
            Ok(vec![
                queued("t1", "First.Release"),
                queued("t2", "Second.Release"),
            ]),
            "the magnet dn value names the torrent"
        );
    }

    #[tokio::test]
    async fn removed_id_is_never_reused() {
        let torrents = FakeTorrents::new();
        torrents.add(&magnet("First.Release")).await.expect("add");
        torrents
            .remove(&TorrentId("t1".to_owned()), false)
            .await
            .expect("remove");
        torrents.add(&magnet("Second.Release")).await.expect("add");

        assert_eq!(
            torrents.list().await,
            Ok(vec![queued("t2", "Second.Release")])
        );
    }

    #[tokio::test]
    async fn fail_next_applies_once() {
        let torrents = FakeTorrents::new();
        torrents.fail_next(TorrentError::Unauthorized);

        assert_eq!(
            torrents.check().await,
            Err(TorrentError::Unauthorized),
            "the scripted error lands on the first call"
        );
        assert_eq!(
            torrents.check().await,
            Ok(ClientInfo {
                version: "fake".to_owned()
            }),
            "the call after it succeeds"
        );
    }

    #[tokio::test]
    async fn duplicate_is_rejected_when_enabled() {
        let torrents = FakeTorrents::new();
        torrents.reject_duplicates();
        torrents.add(&magnet("First.Release")).await.expect("add");

        assert_eq!(
            torrents.add(&magnet("First.Release")).await,
            Err(TorrentError::Rejected {
                reason: "duplicate".to_owned()
            })
        );
        assert_eq!(
            torrents.list().await,
            Ok(vec![queued("t1", "First.Release")]),
            "the rejected add leaves no torrent behind"
        );
    }

    #[tokio::test]
    async fn remove_unknown_is_not_found() {
        let torrents = FakeTorrents::new();

        assert_eq!(
            torrents.remove(&TorrentId("t404".to_owned()), false).await,
            Err(TorrentError::NotFound)
        );
    }

    #[tokio::test]
    async fn set_state_shows_in_list() {
        let torrents = FakeTorrents::new();
        let id = torrents.seed("Existing.Release");
        torrents.set_state(&id, TorrentState::Error("disk full".to_owned()));

        assert_eq!(
            torrents.list().await,
            Ok(vec![Torrent {
                id: TorrentId("t1".to_owned()),
                name: "Existing.Release".to_owned(),
                state: TorrentState::Error("disk full".to_owned()),
                size: 0,
                progress: 1.0,
            }]),
            "seed records no add request and starts seeding"
        );
        assert_eq!(torrents.added(), Vec::new(), "seed is not an add");
    }
}
