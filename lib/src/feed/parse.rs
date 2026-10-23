//! The conversion from what feed-rs produces to what this application needs.
//!
//! This is the only place the feed-rs model appears. Every caller past it
//! sees torrss types, so a change of feed library stops here.
//!
//! A tracker feed is written by someone else and is often wrong. An entry
//! without a title or a usable link is dropped rather than guessed at,
//! because a rule matches on the filename and a client needs the link.
//! One bad entry therefore costs one release, not the whole fetch.

use std::collections::HashMap;

use feed_rs::model::Entry;
use quick_xml::events::Event;
use quick_xml::{Reader, escape};
use url::Url;

use super::{Feed, FeedError, FeedItem};

/// The element whose text [`content_lengths`] reads.
enum Field {
    Guid,
    Length,
}

/// Reads a feed document and returns the releases it announces.
///
/// Accepts any format feed-rs reads, which covers RSS 2.0, RSS 1.0, Atom,
/// and JSON Feed. An entry the application has no use for is dropped, so a
/// successful return holds fewer items than the document has entries.
pub(crate) fn parse(bytes: &[u8]) -> Result<Feed, FeedError> {
    let parsed = feed_rs::parser::parse(bytes).map_err(|error| FeedError::Parse {
        message: error.to_string(),
    })?;
    let lengths = content_lengths(bytes);

    Ok(Feed {
        items: parsed
            .entries
            .into_iter()
            .filter_map(|entry| item(entry, &lengths))
            .collect(),
    })
}

/// Returns the release `entry` announces, or nothing when it is unusable.
fn item(entry: Entry, lengths: &HashMap<String, u64>) -> Option<FeedItem> {
    let link = link(&entry)?;
    let size = size(&entry, lengths);
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
///
/// A feed with no enclosure, such as a UNIT3D tracker feed, states the size
/// in a `contentlength` element that feed-rs drops. The length that
/// [`content_lengths`] reads from the raw document comes last.
fn size(entry: &Entry, lengths: &HashMap<String, u64>) -> Option<u64> {
    media_content(entry)
        .find_map(|content| content.size)
        .or_else(|| entry.links.first().and_then(|link| link.length))
        .or_else(|| lengths.get(&entry.id).copied())
}

fn media_content(entry: &Entry) -> impl Iterator<Item = &feed_rs::model::MediaContent> {
    entry.media.iter().flat_map(|media| &media.content)
}

/// Reads the `contentlength` of each item from the raw document, keyed by
/// the text of the item's `guid`.
///
/// feed-rs models no `contentlength` element, so this pass reads the
/// document a second time. Its key must equal the [`Entry::id`] that feed-rs
/// reads from the same `guid`. So the pass joins text, CDATA, and entity
/// references as feed-rs does, and trims the result.
///
/// An item with no `guid` gets an id that feed-rs makes up, so it gets no
/// length here. The pass stops at the first error and keeps what it read,
/// because [`parse`] runs it only on a document that feed-rs accepted.
fn content_lengths(bytes: &[u8]) -> HashMap<String, u64> {
    let mut reader = Reader::from_reader(bytes);
    let mut lengths = HashMap::new();

    let mut guid: Option<String> = None;
    let mut length: Option<u64> = None;
    let mut field: Option<(Field, String)> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(start)) => match start.local_name().as_ref() {
                "item" => {
                    guid = None;
                    length = None;
                }
                "guid" => field = Some((Field::Guid, String::new())),
                name if name.eq_ignore_ascii_case("contentlength") => {
                    field = Some((Field::Length, String::new()));
                }
                _ => {}
            },

            Ok(Event::End(end)) => {
                match field.take() {
                    Some((Field::Guid, text)) => guid = Some(text.trim().to_owned()),
                    Some((Field::Length, text)) => length = text.trim().parse().ok(),
                    None => {}
                }

                if end.local_name().as_ref() == "item"
                    && let (Some(guid), Some(length)) = (guid.take(), length.take())
                {
                    lengths.insert(guid, length);
                }
            }

            Ok(Event::Eof) | Err(_) => break,

            Ok(event) => {
                if let Some((_, text)) = &mut field {
                    push_text(text, &event);
                }
            }
        }
    }

    lengths
}

/// Appends the text that `event` carries to `text`, as feed-rs joins the
/// text of an element.
///
/// A character reference or a predefined entity resolves to its text. An
/// unknown entity keeps its escaped form, as feed-rs keeps it.
fn push_text(text: &mut String, event: &Event<'_>) {
    match event {
        Event::Text(content) => text.push_str(content),
        Event::CData(content) => text.push_str(content),
        Event::GeneralRef(reference) => match reference.resolve_char_ref() {
            Ok(Some(character)) => text.push(character),
            _ => match escape::resolve_predefined_entity(reference) {
                Some(resolved) => text.push_str(resolved),
                None => {
                    text.push('&');
                    text.push_str(reference);
                    text.push(';');
                }
            },
        },
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::parse;
    use crate::feed::{Feed, FeedError, FeedItem};

    const TRACKER: &[u8] = include_bytes!("fixtures/tracker.xml");
    const CONTENT_LENGTH: &[u8] = include_bytes!("fixtures/contentlength.xml");

    fn tracker() -> Feed {
        parse(TRACKER).expect("the tracker fixture parses")
    }

    /// Parses an RSS document around `items` and returns the size of each
    /// item.
    ///
    /// The document declares the ezrss `torrent` namespace for an item that
    /// carries its elements.
    fn sizes(items: &str) -> Vec<Option<u64>> {
        let document = format!(
            r#"<?xml version="1.0"?>
            <rss version="2.0" xmlns:torrent="http://xmlns.ezrss.it/0.1/"><channel>
              <title>Invented Tracker</title>
              {items}
            </channel></rss>"#
        );

        parse(document.as_bytes())
            .expect("the inline feed parses")
            .items
            .into_iter()
            .map(|item| item.size)
            .collect()
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
    fn content_length_element_gives_size() {
        assert_eq!(
            parse(CONTENT_LENGTH)
                .expect("the contentlength fixture parses")
                .items,
            [FeedItem {
                guid: "invented-series-s02".to_owned(),
                title: "Invented.Series.S02.1080p.BluRay.x265-GROUP".to_owned(),
                link: "https://tracker.invalid/torrent/download/invented-series-s02.key"
                    .parse()
                    .expect("the fixture link parses"),
                published: Some(
                    Utc.with_ymd_and_hms(2025, 3, 6, 18, 30, 0)
                        .single()
                        .expect("the fixture date is unambiguous")
                ),
                size: Some(4_209_067_622),
                seeders: None,
            }],
            "a UNIT3D feed states the size in a contentlength element"
        );
    }

    #[test]
    fn namespaced_content_length_gives_size() {
        assert_eq!(
            sizes(
                r#"<item>
                  <title>Invented.Series.S01.1080p.WEB-DL.x264-GROUP</title>
                  <link>https://tracker.invalid/torrent/download/invented-series-s01.key</link>
                  <guid>invented-series-s01</guid>
                  <torrent:contentLength>1024</torrent:contentLength>
                </item>"#
            ),
            [Some(1024)],
            "an ezrss feed states the size in a namespaced contentLength element"
        );
    }

    #[test]
    fn enclosure_length_wins_over_content_length() {
        assert_eq!(
            sizes(
                r#"<item>
                  <title>Invented.Series.S03.1080p.WEB-DL.x264-GROUP</title>
                  <guid>invented-series-s03</guid>
                  <enclosure url="https://tracker.invalid/torrent/download/invented-series-s03.key"
                    length="2048" type="application/x-bittorrent"/>
                  <contentlength>1</contentlength>
                </item>"#
            ),
            [Some(2048)],
            "the enclosure states the size feed-rs already read"
        );
    }

    #[test]
    fn escaped_or_cdata_guid_gives_size() {
        assert_eq!(
            sizes(
                r#"<item>
                  <title>Invented.Series.S04.1080p.WEB-DL.x264-GROUP</title>
                  <link>https://tracker.invalid/torrent/download/invented-series-s04.key</link>
                  <guid>https://tracker.invalid/torrents?id=4&amp;season=4</guid>
                  <contentlength>512</contentlength>
                </item>
                <item>
                  <title>Invented.Series.S05.1080p.WEB-DL.x264-GROUP</title>
                  <link>https://tracker.invalid/torrent/download/invented-series-s05.key</link>
                  <guid><![CDATA[invented-series-s05]]></guid>
                  <contentlength>256</contentlength>
                </item>"#
            ),
            [Some(512), Some(256)],
            "a guid keys its length as feed-rs reads the guid"
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
