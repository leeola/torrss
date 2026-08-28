//! The torrent client that downloads a matched release.
//!
//! A tracker feed only names a release. Something has to download it.
//! [`TorrentClient`] is the whole surface the application uses to add, list,
//! and remove torrents, so the qBittorrent wire format never reaches a
//! handler or a rule.
//!
//! [`TorrentError`] carries plain data rather than the client library's error
//! type. A test fake then produces any failure the application handles,
//! without a live connection to fail against.

use async_trait::async_trait;
use snafu::Snafu;
use url::Url;

/// Where the torrent data comes from.
///
/// A tracker publishes a release as a magnet link, as a `.torrent` URL, or
/// occasionally as the file itself, and the client accepts all three. The
/// variant is kept rather than resolved, because a magnet link and a URL take
/// different paths through the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TorrentSource {
    Magnet(Url),
    Url(Url),
    File { filename: String, data: Vec<u8> },
}

/// One request to start downloading a release.
///
/// The category and tags are how a rule marks what it matched, so the user
/// sorts the client's queue by the rule that filled it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddTorrent {
    pub source: TorrentSource,
    pub category: Option<String>,
    pub tags: Vec<String>,
    /// Adds the torrent without starting it, for a release the user confirms
    /// by hand before it uses bandwidth.
    pub paused: bool,
}

impl AddTorrent {
    /// Returns a request that adds `source` at once, with no category or tag.
    ///
    /// [`TorrentSource`] has no meaningful empty value, so the type carries no
    /// [`Default`] and this constructor supplies the remaining fields.
    pub fn new(source: TorrentSource) -> Self {
        Self {
            source,
            category: None,
            tags: Vec::new(),
            paused: false,
        }
    }
}

/// The info hash the client files a torrent under.
///
/// The client assigns it, so it is opaque here and only ever echoed back.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TorrentId(pub String);

/// One torrent as the client currently reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct Torrent {
    pub id: TorrentId,
    pub name: String,
    pub state: TorrentState,
    pub size: u64,
    /// Completed fraction, from 0.0 to 1.0.
    pub progress: f32,
}

/// How far along a torrent is.
///
/// qBittorrent reports more than a dozen states that differ only in why a
/// transfer stalled. They collapse to these five, because the application
/// reports progress rather than diagnoses the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TorrentState {
    Queued,
    Downloading,
    Seeding,
    Paused,
    Error(String),
}

/// What a reachable client reports about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientInfo {
    pub version: String,
}

/// Why a torrent client request did not succeed.
#[derive(Debug, Clone, PartialEq, Eq, Snafu)]
pub enum TorrentError {
    /// The client did not answer at all. The client is down, or the
    /// configured address is wrong.
    #[snafu(display("the torrent client is unreachable: {message}"))]
    Unreachable { message: String },

    /// The client refused the credentials, or banned this address for trying.
    #[snafu(display("the torrent client rejected the credentials"))]
    Unauthorized,

    /// The client understood the request and declined it. A duplicate torrent
    /// and an unwritable save path both land here.
    #[snafu(display("the torrent client rejected the request: {reason}"))]
    Rejected { reason: String },

    /// No torrent carries the requested id. A torrent removed outside the
    /// application produces this.
    #[snafu(display("the torrent client has no such torrent"))]
    NotFound,

    /// The client answered in a shape this adapter does not understand,
    /// which points at a version mismatch rather than at the request.
    #[snafu(display("the torrent client answered unexpectedly: {message}"))]
    Protocol { message: String },
}

/// A torrent client the application drives.
///
/// The trait is the whole contract. One implementation talks to qBittorrent
/// over HTTP. Another records the calls in memory for a test.
#[async_trait]
pub trait TorrentClient: Send + Sync {
    /// Adds one torrent and returns once the client accepts it.
    ///
    /// No id comes back, because the qBittorrent add endpoint answers with an
    /// empty body. Call [`Self::list`] to find the torrent this created.
    async fn add(&self, request: &AddTorrent) -> Result<(), TorrentError>;

    /// Returns every torrent the client holds, in the client's own order.
    async fn list(&self) -> Result<Vec<Torrent>, TorrentError>;

    /// Removes one torrent, and its downloaded data when `delete_files` is set.
    ///
    /// Returns [`TorrentError::NotFound`] when no torrent carries `id`.
    async fn remove(&self, id: &TorrentId, delete_files: bool) -> Result<(), TorrentError>;

    /// Confirms the client is reachable and the credentials work.
    ///
    /// The admin page calls this to report a connection status. The error it
    /// returns carries the value here, not the work it does.
    async fn check(&self) -> Result<ClientInfo, TorrentError>;
}
