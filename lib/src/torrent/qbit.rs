//! The qBittorrent adapter.
//!
//! This is the only place the qbit-rs model appears. The four mapping
//! functions below hold every decision about how the wire shapes become
//! torrss types, and the trait impl only calls the client and runs them.
//!
//! qBittorrent reports around twenty transfer states and a torrent whose
//! every field is optional. Narrowing that to the handful the application
//! shows is the work here, and it is why the mappings carry the tests
//! rather than the trait impl.

use async_trait::async_trait;
use chrono::DateTime;
use qbit_rs::model::{
    AddTorrentArg, Credential, GetTorrentListArg, Sep, State, Torrent as QbitTorrent, TorrentFile,
    TorrentSource as QbitSource,
};
use qbit_rs::{ApiError, Error as QbitError, Qbit as QbitClient};
use url::Url;

use super::{
    AddTorrent, ClientInfo, Torrent, TorrentClient, TorrentError, TorrentId, TorrentSource,
    TorrentState,
};

/// A qBittorrent web API client.
pub struct Qbit {
    client: QbitClient,
}

impl Qbit {
    /// Returns a client for the qBittorrent at `endpoint`.
    ///
    /// No request happens here. The credentials are checked on the first
    /// call, so a wrong password surfaces from [`TorrentClient::check`]
    /// rather than from construction.
    pub fn new(endpoint: Url, username: &str, password: &str) -> Self {
        Self {
            client: QbitClient::new(endpoint, Credential::new(username, password)),
        }
    }
}

#[async_trait]
impl TorrentClient for Qbit {
    async fn add(&self, request: &AddTorrent) -> Result<(), TorrentError> {
        self.client
            .add_torrent(add_arg(request))
            .await
            .map_err(error)
    }

    async fn list(&self) -> Result<Vec<Torrent>, TorrentError> {
        let raw = self
            .client
            .get_torrent_list(GetTorrentListArg::default())
            .await
            .map_err(error)?;

        Ok(raw.into_iter().filter_map(torrent).collect())
    }

    async fn check(&self) -> Result<ClientInfo, TorrentError> {
        let version = self.client.get_version().await.map_err(error)?;

        Ok(ClientInfo { version })
    }
}

/// Builds the add request qBittorrent expects.
///
/// Both `paused` and `stopped` are set together. qBittorrent renamed the
/// field at version 5.0, and setting both means one request works against
/// either side of that rename.
fn add_arg(request: &AddTorrent) -> AddTorrentArg {
    let paused = request.paused.then(|| "true".to_owned());

    AddTorrentArg {
        source: source(&request.source),
        category: request.category.clone(),
        // An empty string here asks qBittorrent to create a blank tag.
        tags: (!request.tags.is_empty()).then(|| request.tags.join(",")),
        paused: paused.clone(),
        stopped: paused,
        ..Default::default()
    }
}

fn source(source: &TorrentSource) -> QbitSource {
    match source {
        TorrentSource::Magnet(url) | TorrentSource::Url(url) => QbitSource::Urls {
            urls: Sep::from(vec![url.clone()]),
        },
        TorrentSource::File { filename, data } => QbitSource::TorrentFiles {
            torrents: vec![TorrentFile {
                filename: filename.clone(),
                data: data.clone(),
            }],
        },
    }
}

/// Collapses a qBittorrent transfer state into one the application shows.
///
/// Most of the reported states differ only in why a transfer is not moving,
/// which the application does not report. A failed state keeps the wire
/// name, because that is the only detail qBittorrent gives about it.
fn state(state: Option<State>) -> TorrentState {
    match state {
        Some(State::Downloading | State::ForcedDL | State::StalledDL) => TorrentState::Downloading,

        Some(
            State::Uploading
            | State::ForcedUP
            | State::StalledUP
            | State::QueuedUP
            | State::CheckingUP,
        ) => TorrentState::Seeding,

        Some(State::PausedDL | State::PausedUP) => TorrentState::Paused,

        Some(
            State::QueuedDL
            | State::Allocating
            | State::MetaDL
            | State::CheckingDL
            | State::CheckingResumeData
            | State::Moving,
        ) => TorrentState::Queued,

        Some(State::Error) => TorrentState::Error("error".to_owned()),
        Some(State::MissingFiles) => TorrentState::Error("missingFiles".to_owned()),
        Some(State::Unknown) | None => TorrentState::Error("unknown".to_owned()),
    }
}

/// Converts one reported torrent, and drops an unidentifiable one.
///
/// Every field qBittorrent reports is optional. A torrent with no hash or
/// no name names nothing the application acts on, so it is dropped rather
/// than shown as a blank row.
fn torrent(raw: QbitTorrent) -> Option<Torrent> {
    let id = TorrentId(raw.hash?);
    let name = raw.name?;

    Some(Torrent {
        id,
        name,
        // qBittorrent reports -1 for a size it has not learned yet.
        size: raw.total_size.or(raw.size).unwrap_or(0).max(0) as u64,
        progress: raw.progress.unwrap_or(0.0) as f32,
        state: state(raw.state),
        added_at: raw
            .added_on
            .and_then(|seconds| DateTime::from_timestamp(seconds, 0)),
    })
}

/// Sorts a qbit-rs failure into the kind the application acts on.
///
/// The application retries an unreachable client, asks the user to fix an
/// unauthorized one, and reports the rest. Anything the adapter does not
/// recognize is a protocol failure, which points at a version mismatch.
fn error(error: QbitError) -> TorrentError {
    match &error {
        QbitError::ApiError(
            ApiError::BadCredentials | ApiError::NotLoggedIn | ApiError::IpBanned,
        ) => TorrentError::Unauthorized,

        QbitError::ApiError(
            ApiError::TorrentAddFailed
            | ApiError::TorrentFileInvalid
            | ApiError::TorrentNameEmpty
            | ApiError::SavePathEmpty
            | ApiError::NoWriteAccess
            | ApiError::UnableToCreateDir
            | ApiError::CategoryNotFound,
        ) => TorrentError::Rejected {
            reason: error.to_string(),
        },

        QbitError::HttpError(http) if http.is_connect() || http.is_timeout() => {
            TorrentError::Unreachable {
                message: error.to_string(),
            }
        }

        _ => TorrentError::Protocol {
            message: error.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use chrono::DateTime;
    use qbit_rs::model::{Sep, State, Torrent as QbitTorrent, TorrentSource as QbitSource};
    use qbit_rs::{ApiError, Error as QbitError};
    use url::Url;

    use super::{add_arg, error, state, torrent};
    use crate::torrent::{
        AddTorrent, Torrent, TorrentError, TorrentId, TorrentSource, TorrentState,
    };

    const MAGNET: &str = "magnet:?xt=urn:btih:0123456789abcdef&dn=Invented.Release";
    const TORRENT_URL: &str = "https://tracker.invalid/invented.torrent";

    fn url(raw: &str) -> Url {
        Url::parse(raw).expect("the test URL parses")
    }

    #[test]
    fn magnet_and_url_become_urls() {
        for raw in [MAGNET, TORRENT_URL] {
            let source = if raw == MAGNET {
                TorrentSource::Magnet(url(raw))
            } else {
                TorrentSource::Url(url(raw))
            };

            assert_eq!(
                add_arg(&AddTorrent::new(source)).source,
                QbitSource::Urls {
                    urls: Sep::from(vec![url(raw)])
                },
                "{raw} reaches qBittorrent as a URL"
            );
        }
    }

    #[test]
    fn paused_sets_both_flags() {
        let mut request = AddTorrent::new(TorrentSource::Magnet(url(MAGNET)));
        request.category = Some("shows".to_owned());
        request.tags = vec!["rule-a".to_owned(), "rule-b".to_owned()];
        request.paused = true;

        let arg = add_arg(&request);

        assert_eq!(arg.paused.as_deref(), Some("true"), "the pre-5.0 field");
        assert_eq!(arg.stopped.as_deref(), Some("true"), "the 5.0 field");
        assert_eq!(arg.category.as_deref(), Some("shows"));
        assert_eq!(arg.tags.as_deref(), Some("rule-a,rule-b"));

        let running = add_arg(&AddTorrent::new(TorrentSource::Magnet(url(MAGNET))));

        assert_eq!(running.paused, None, "an unpaused add sets neither flag");
        assert_eq!(running.stopped, None, "an unpaused add sets neither flag");
        assert_eq!(running.tags, None, "no tag beats a blank tag");
    }

    #[test]
    fn state_groups_map() {
        let cases = [
            (Some(State::Downloading), TorrentState::Downloading),
            (Some(State::StalledDL), TorrentState::Downloading),
            (Some(State::Uploading), TorrentState::Seeding),
            (Some(State::QueuedUP), TorrentState::Seeding),
            (Some(State::PausedDL), TorrentState::Paused),
            (Some(State::Moving), TorrentState::Queued),
            (Some(State::Error), TorrentState::Error("error".to_owned())),
            (
                Some(State::MissingFiles),
                TorrentState::Error("missingFiles".to_owned()),
            ),
            (
                Some(State::Unknown),
                TorrentState::Error("unknown".to_owned()),
            ),
            (None, TorrentState::Error("unknown".to_owned())),
        ];

        for (reported, expected) in cases {
            assert_eq!(state(reported.clone()), expected, "{reported:?}");
        }
    }

    #[test]
    fn torrent_without_hash_is_dropped() {
        let blank: QbitTorrent =
            serde_json::from_str("{}").expect("every reported field is optional");

        assert_eq!(torrent(blank.clone()), None, "no hash and no name");

        let named = QbitTorrent {
            name: Some("Invented.Release".to_owned()),
            ..blank.clone()
        };

        assert_eq!(torrent(named), None, "a name alone identifies nothing");

        let whole = QbitTorrent {
            hash: Some("abc123".to_owned()),
            name: Some("Invented.Release".to_owned()),
            total_size: Some(-1),
            size: Some(2048),
            progress: Some(0.5),
            state: Some(State::StalledUP),
            added_on: Some(1_700_000_000),
            ..blank
        };

        assert_eq!(
            torrent(whole),
            Some(Torrent {
                id: TorrentId("abc123".to_owned()),
                name: "Invented.Release".to_owned(),
                state: TorrentState::Seeding,
                size: 0,
                progress: 0.5,
                added_at: DateTime::from_timestamp(1_700_000_000, 0),
            }),
            "an unknown total size clamps to zero rather than wrapping"
        );
    }

    #[test]
    fn bad_credentials_is_unauthorized() {
        assert_eq!(
            error(QbitError::ApiError(ApiError::BadCredentials)),
            TorrentError::Unauthorized
        );
        assert!(
            matches!(
                error(QbitError::ApiError(ApiError::CategoryNotFound)),
                TorrentError::Rejected { .. }
            ),
            "a refused request is rejected rather than unauthorized"
        );
        assert!(
            matches!(
                error(QbitError::NonAsciiHeader),
                TorrentError::Protocol { .. }
            ),
            "an unrecognized failure points at the protocol"
        );
    }
}
