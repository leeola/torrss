//! The conversion from what feed-rs produces to what this application needs.
//!
//! This is the only place the feed-rs model appears. Every caller past it
//! sees torrss types, so a change of feed library stops here.
//!
//! A tracker feed is written by someone else and is often wrong. An entry
//! without a title or a usable link is dropped rather than guessed at,
//! because a rule matches on the filename and a client needs the link.
//! One bad entry therefore costs one release, not the whole fetch.

use feed_rs::model::Entry;
use url::Url;

use super::{Feed, FeedError, FeedItem};

/// Reads a feed document and returns the releases it announces.
///
/// Accepts any format feed-rs reads, which covers RSS 2.0, RSS 1.0, Atom,
/// and JSON Feed. An entry the application has no use for is dropped, so a
/// successful return holds fewer items than the document has entries.
pub(crate) fn parse(bytes: &[u8]) -> Result<Feed, FeedError> {
    let parsed = feed_rs::parser::parse(bytes).map_err(|error| FeedError::Parse {
        message: error.to_string(),
    })?;

    Ok(Feed {
        items: parsed.entries.into_iter().filter_map(item).collect(),
    })
}

/// Returns the release `entry` announces, or nothing when it is unusable.
fn item(entry: Entry) -> Option<FeedItem> {
    let link = link(&entry)?;
    let size = size(&entry);
    let title = entry.title?.content;

    Some(FeedItem {
        guid: entry.id,
        title,
        link,
        published: entry.published,
        size,
        // FIXME: feed-rs drops the torznab attributes an indexer reports the
        // seeder count in, so a torznab feed never fills this field.
        seeders: None,
    })
}

/// Returns where to get the torrent data for `entry`.
///
/// An Atom or RSS `<link>` wins when the entry has one. An RSS
/// `<enclosure>` reaches feed-rs as media content instead, which is the
/// only place a plain RSS tracker states the torrent URL.
fn link(entry: &Entry) -> Option<Url> {
    match entry.links.first() {
        Some(link) => Url::parse(&link.href).ok(),
        None => media_content(entry).find_map(|content| content.url.clone()),
    }
}

/// Returns the release size in bytes for `entry`.
///
/// An enclosure states the size beside the URL, so media content is read
/// first. An Atom link states it separately, and often not at all.
fn size(entry: &Entry) -> Option<u64> {
    media_content(entry)
        .find_map(|content| content.size)
        .or_else(|| entry.links.first().and_then(|link| link.length))
}

fn media_content(entry: &Entry) -> impl Iterator<Item = &feed_rs::model::MediaContent> {
    entry.media.iter().flat_map(|media| &media.content)
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::parse;
    use crate::feed::{Feed, FeedError, FeedItem};

    const TRACKER: &[u8] = include_bytes!("fixtures/tracker.xml");

    fn tracker() -> Feed {
        parse(TRACKER).expect("the tracker fixture parses")
    }

    #[test]
    fn enclosure_gives_url_and_size() {
        let feed = tracker();

        assert_eq!(feed.items.len(), 2, "item count");
        assert_eq!(
            feed.items[0],
            FeedItem {
                guid: "invented-show-s01e01".to_owned(),
                title: "Invented.Show.S01E01.1080p.WEB-DL.x264-GROUP".to_owned(),
                link: "https://tracker.invalid/torrents/invented-show-s01e01.torrent"
                    .parse()
                    .expect("the fixture enclosure URL parses"),
                published: Some(
                    Utc.with_ymd_and_hms(2025, 3, 4, 9, 15, 0)
                        .single()
                        .expect("the fixture date is unambiguous")
                ),
                size: Some(734_003_200),
                seeders: None,
            }
        );
    }

    #[test]
    fn magnet_link_parses_as_url() {
        assert_eq!(
            tracker().items[1],
            FeedItem {
                guid: "invented-movie-2024".to_owned(),
                title: "Invented.Movie.2024.2160p.BluRay.x265-GROUP".to_owned(),
                link: "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Invented.Movie.2024"
                    .parse()
                    .expect("the fixture magnet link parses"),
                published: Some(
                    Utc.with_ymd_and_hms(2025, 3, 5, 21, 40, 0)
                        .single()
                        .expect("the fixture date is unambiguous")
                ),
                size: None,
                seeders: None,
            }
        );
    }

    #[test]
    fn entry_without_title_is_skipped() {
        let untitled = br#"<?xml version="1.0"?>
            <rss version="2.0"><channel>
              <title>Invented Tracker</title>
              <item>
                <guid isPermaLink="false">no-title</guid>
                <link>https://tracker.invalid/torrents/no-title.torrent</link>
              </item>
            </channel></rss>"#;

        assert_eq!(parse(untitled), Ok(Feed { items: Vec::new() }));
    }

    #[test]
    fn malformed_xml_is_parse_error() {
        let result = parse(b"<rss><channel><item></rss>");

        assert!(
            matches!(result, Err(FeedError::Parse { .. })),
            "malformed XML gives a parse error, got {result:?}"
        );
    }
}
