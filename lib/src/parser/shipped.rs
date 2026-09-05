//! The parsers the binary carries.
//!
//! A fresh database stores none, so these are what read a title until the
//! reader writes one of their own. They are declared in the order the engine
//! tries them, most specific first, because
//! [`Engine::read`](crate::rules::Engine::read) takes the first parser that
//! reads a name.
//!
//! Each names its fields as [`super::PRESETS`] does. An import and a ruleset
//! therefore find `show`, `season`, `episodeNumber`, and `resolution` under
//! one name, whichever parser read them.
//!
//! Every separator is `[. _]`. A tracker writes a name in dots, and a client
//! writes the folder it unpacked into in spaces.

use super::FieldKind::{Enum, Episode, Number, Season, Text};
use super::{Field, FieldKind, Parser};

/// The parsers the binary carries, in the order the engine tries them.
pub(crate) fn parsers() -> Vec<Parser> {
    // The episode name is a lazy run that nothing inside it ends, so the
    // resolution behind it is what stops it.
    let series_tags = tags().into_iter().map(|tag| Field {
        required: tag.name == "resolution",
        ..tag
    });

    vec![
        // The show is lazy, so it stops at the first season token. The year
        // is optional and takes no part in the identity, so Show.2019.S02E05
        // and Show.S02E05 read one show rather than two. A double episode
        // reads its first number, because the pair is one file.
        //
        // The episode name is a lazy run with nothing of its own to end it,
        // so the required resolution is what stops it. That is why a title
        // naming no resolution falls through to series-loose. It also means a
        // tag written between the episode and the resolution, such as REPACK,
        // reads as the episode name, which no import conditions on.
        Parser {
            id: "series".to_owned(),
            name: "Series".to_owned(),
            fields: vec![
                field("show", Text, Some(r"^(?<show>.+?)"), true, true, true),
                field(
                    "year",
                    Number,
                    Some(r"[. _](?<year>(?:19|20)\d{2})"),
                    false,
                    false,
                    true,
                ),
                field("season", Season, None, true, true, true),
                field(
                    "episodeNumber",
                    Episode,
                    Some(r"(?i)E(?<episodeNumber>\d{1,3})(?:-?E\d{1,3})?"),
                    false,
                    true,
                    true,
                ),
                field(
                    "episodeName",
                    Text,
                    Some(r"[. _](?<episodeName>.+?)"),
                    false,
                    false,
                    true,
                ),
            ]
            .into_iter()
            .chain(series_tags)
            .collect(),
            tests: Vec::new(),
            built_in: true,
        },
        // This one claims no episode name. The episode is not tight, so the
        // gap after it skips whatever name the title carries, and nothing
        // here has to end a run. That is what series requires its resolution
        // for, so a title naming none reads here instead.
        //
        // The identity fields match series, so an episode files into
        // show+season+episodeNumber whichever of the two read it.
        Parser {
            id: "series-loose".to_owned(),
            name: "Series without resolution".to_owned(),
            fields: vec![
                field("show", Text, Some(r"^(?<show>.+?)"), true, true, true),
                field(
                    "year",
                    Number,
                    Some(r"[. _](?<year>(?:19|20)\d{2})"),
                    false,
                    false,
                    true,
                ),
                field("season", Season, None, true, true, true),
                field(
                    "episodeNumber",
                    Episode,
                    Some(r"(?i)E(?<episodeNumber>\d{1,3})(?:-?E\d{1,3})?"),
                    false,
                    true,
                    false,
                ),
            ]
            .into_iter()
            .chain(tags())
            .collect(),
            tests: Vec::new(),
            built_in: true,
        },
        // The season keeps the Season kind, so it normalizes as a number and
        // this episode files with the S02E05 spelling of the same one. The x
        // guards the digits, so a codec such as x265 never reads as a season.
        Parser {
            id: "series-x".to_owned(),
            name: "Series numbered 2x05".to_owned(),
            fields: vec![
                field("show", Text, Some(r"^(?<show>.+?)"), true, true, true),
                field(
                    "season",
                    Season,
                    Some(r"[. _](?<season>\d{1,2})[xX]"),
                    true,
                    true,
                    true,
                ),
                field(
                    "episodeNumber",
                    Episode,
                    Some(r"(?<episodeNumber>\d{2,3})"),
                    true,
                    true,
                    false,
                ),
            ]
            .into_iter()
            .chain(tags())
            .collect(),
            tests: Vec::new(),
            built_in: true,
        },
    ]
}

/// Builds one field, so a parser above reads as a list of rules rather than
/// a page of struct literals.
fn field(
    name: &str,
    kind: FieldKind,
    pattern: Option<&str>,
    required: bool,
    identity: bool,
    tight: bool,
) -> Field {
    Field {
        name: name.to_owned(),
        kind,
        pattern: pattern.map(ToOwned::to_owned),
        required,
        tight,
        identity,
    }
}

/// The trailing fields every shipped parser ends with.
///
/// These say how a release was encoded rather than what it is, so none takes
/// part in the identity. A tracker writes as few of them as it likes and in
/// its own order, so each is optional and none is tight.
fn tags() -> Vec<Field> {
    vec![
        field(
            "resolution",
            Enum,
            Some(r"(?i)[. _](?<resolution>480p|576p|720p|1080p|1080i|2160p)"),
            false,
            false,
            false,
        ),
        // WEB closes the alternation because the regex crate takes the first
        // alternative that matches, and WEB ahead of WEB-DL cuts it short.
        field(
            "source",
            Enum,
            Some(
                r"(?i)[. _](?<source>WEB-?DL|WEB-?Rip|BluRay|Blu-Ray|BDRip|BRRip|HDTV|DVDRip|REMUX|WEB)",
            ),
            false,
            false,
            false,
        ),
        field(
            "codec",
            Enum,
            Some(r"(?i)[. _](?<codec>[xh][. ]?26[45]|HEVC|AVC|AV1|XviD)"),
            false,
            false,
            false,
        ),
        field(
            "audio",
            Text,
            Some(
                r"(?i)[. _](?<audio>DDP?[. ]?\d[. ]\d|EAC3|AC3|DTS(?:-HD)?|TrueHD|Atmos|AAC|FLAC|Opus)",
            ),
            false,
            false,
            false,
        ),
        field(
            "publisher",
            Text,
            Some(r".*-(?<publisher>[A-Za-z0-9]+)"),
            false,
            false,
            false,
        ),
        field(
            "extension",
            Enum,
            Some(r"(?i)(?<extension>\.(?:mkv|mp4|avi))$"),
            false,
            false,
            false,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::LazyLock;

    use super::parsers;
    use crate::rules::Engine;
    use crate::ruleset::Ruleset;

    /// The shipped set compiled, which is what proves every pattern above is
    /// a valid regex.
    ///
    /// # Panics
    ///
    /// Panics when a pattern fails to compile, which makes a bad shipped
    /// pattern a failure of the test run rather than a silent miss.
    static ENGINE: LazyLock<Engine> = LazyLock::new(|| {
        Engine::new(parsers(), Vec::new()).expect("every shipped pattern is a valid regex")
    });

    /// The whole reading the shipped set makes of one title.
    ///
    /// It names the parser that read the title, then each field's name beside
    /// the value it read, in the parser's own order. [`None`] is a title no
    /// shipped parser reads at all.
    type Read<'a> = Option<(&'a str, &'a [(&'a str, &'a str)])>;

    /// A title, and the whole reading the shipped set makes of it.
    ///
    /// Every row names every value the parser captured, so a pattern that
    /// claims one run too many fails here rather than passing unseen.
    const READINGS: &[(&str, Read<'_>)] = &[
        (
            "Coastal.Ecology.S02E05.1080p.WEB-DL.x265.DDP5.1-OpenReel.mkv",
            Some((
                "series",
                &[
                    ("show", "coastal ecology"),
                    ("season", "2"),
                    ("episodeNumber", "5"),
                    ("resolution", "1080p"),
                    ("source", "web dl"),
                    ("codec", "x265"),
                    ("audio", "ddp5 1"),
                    ("publisher", "openreel"),
                    ("extension", "mkv"),
                ],
            )),
        ),
        (
            "Coastal.Ecology.S02E05.The.Tide.Line.1080p.WEB-DL-OpenReel.mkv",
            Some((
                "series",
                &[
                    ("show", "coastal ecology"),
                    ("season", "2"),
                    ("episodeNumber", "5"),
                    ("episodeName", "the tide line"),
                    ("resolution", "1080p"),
                    ("source", "web dl"),
                    ("publisher", "openreel"),
                    ("extension", "mkv"),
                ],
            )),
        ),
        (
            "Coastal Ecology S02E05 1080p HDTV-PublicWave",
            Some((
                "series",
                &[
                    ("show", "coastal ecology"),
                    ("season", "2"),
                    ("episodeNumber", "5"),
                    ("resolution", "1080p"),
                    ("source", "hdtv"),
                    ("publisher", "publicwave"),
                ],
            )),
        ),
        (
            "The.Hollow.Meridian.2019.S04E06.720p.BluRay.x264-MeridianPress",
            Some((
                "series",
                &[
                    ("show", "the hollow meridian"),
                    ("year", "2019"),
                    ("season", "4"),
                    ("episodeNumber", "6"),
                    ("resolution", "720p"),
                    ("source", "bluray"),
                    ("codec", "x264"),
                    ("publisher", "meridianpress"),
                ],
            )),
        ),
        (
            "Ashfall.County.S03.1080p.WEB-DL-PublicWave",
            Some((
                "series",
                &[
                    ("show", "ashfall county"),
                    ("season", "3"),
                    ("resolution", "1080p"),
                    ("source", "web dl"),
                    ("publisher", "publicwave"),
                ],
            )),
        ),
        (
            "Ridge.Runner.S01E05E06.1080p.WEB",
            Some((
                "series",
                &[
                    ("show", "ridge runner"),
                    ("season", "1"),
                    ("episodeNumber", "5"),
                    ("resolution", "1080p"),
                    ("source", "web"),
                ],
            )),
        ),
        (
            "Ridge.Runner.S01E05.1080p.The.Tide.Line.WEB",
            Some((
                "series",
                &[
                    ("show", "ridge runner"),
                    ("season", "1"),
                    ("episodeNumber", "5"),
                    ("resolution", "1080p"),
                    ("source", "web"),
                ],
            )),
        ),
        (
            "Ashfall.County.S03E04.HDTV.XviD-PublicWave.avi",
            Some((
                "series-loose",
                &[
                    ("show", "ashfall county"),
                    ("season", "3"),
                    ("episodeNumber", "4"),
                    ("source", "hdtv"),
                    ("codec", "xvid"),
                    ("publisher", "publicwave"),
                    ("extension", "avi"),
                ],
            )),
        ),
        (
            "Ashfall.County.S03E04.The.Tide.Line.HDTV-PublicWave",
            Some((
                "series-loose",
                &[
                    ("show", "ashfall county"),
                    ("season", "3"),
                    ("episodeNumber", "4"),
                    ("source", "hdtv"),
                    ("publisher", "publicwave"),
                ],
            )),
        ),
        (
            "Ridge.Runner.2x05.720p.HDTV-OpenReel.mkv",
            Some((
                "series-x",
                &[
                    ("show", "ridge runner"),
                    ("season", "2"),
                    ("episodeNumber", "5"),
                    ("resolution", "720p"),
                    ("source", "hdtv"),
                    ("publisher", "openreel"),
                    ("extension", "mkv"),
                ],
            )),
        ),
        (
            "Ridge Runner 12x105 1080p",
            Some((
                "series-x",
                &[
                    ("show", "ridge runner"),
                    ("season", "12"),
                    ("episodeNumber", "105"),
                    ("resolution", "1080p"),
                ],
            )),
        ),
        ("just some words with no structure at all", None),
    ];

    #[test]
    fn every_title_reads_through_its_parser() {
        let read = READINGS
            .iter()
            .map(|(title, _)| (*title, reads(title)))
            .collect::<Vec<_>>();

        let expected = READINGS
            .iter()
            .map(|(title, reading)| {
                let reading = reading.map(|(parser, values)| {
                    let values = values
                        .iter()
                        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                        .collect();

                    (parser.to_owned(), values)
                });

                (*title, reading)
            })
            .collect::<Vec<_>>();

        assert_eq!(read, expected);
    }

    #[test]
    fn a_title_without_a_resolution_shares_the_identity_of_one_with() {
        let engine = Engine::new(
            parsers(),
            vec![
                claiming("episodes", "series"),
                claiming("loose", "series-loose"),
            ],
        )
        .expect("every shipped pattern is a valid regex");

        let identity = |title: &str| {
            engine
                .parse(title)
                .unwrap_or_else(|| panic!("{title} is claimed"))
                .identity
        };

        assert_eq!(
            identity("Ashfall.County.S03E04.1080p.WEB"),
            identity("Ashfall.County.S03E04.HDTV"),
            "one tracker names the resolution and another does not, and it is one episode"
        );
    }

    /// A ruleset on `parser` that writes no condition, so it claims every
    /// title the parser reads.
    fn claiming(id: &str, parser: &str) -> Ruleset {
        Ruleset {
            id: id.to_owned(),
            name: id.to_owned(),
            enabled: true,
            parser: parser.to_owned(),
            conditions: Vec::new(),
            tests: Vec::new(),
        }
    }

    #[test]
    fn a_codec_is_never_a_season() {
        assert_ne!(
            reads("Coastal.Drift.2024.1080p.x265-OpenReel").map(|(parser, _)| parser),
            Some("series-x".to_owned()),
            "the x of x265 follows no digits, so the 2x05 numbering never claims it"
        );
    }

    #[test]
    fn every_shipped_parser_is_built_in() {
        assert!(parsers().iter().all(|parser| parser.built_in));
    }

    #[test]
    fn shipped_ids_are_distinct() {
        let ids = parsers()
            .into_iter()
            .map(|parser| parser.id)
            .collect::<Vec<_>>();

        assert_eq!(
            ids.iter().collect::<BTreeSet<_>>().len(),
            ids.len(),
            "the engine reads by the first parser of an id: {ids:?}"
        );
    }

    /// Reads `title` through the shipped set, as the parser that read it and
    /// each value normalized by its field's kind.
    fn reads(title: &str) -> Option<(String, Vec<(String, String)>)> {
        let reading = ENGINE.read(title)?;

        let fields = &ENGINE
            .parser(&reading.parser)
            .expect("the engine read with a parser it carries")
            .fields;

        let values = reading
            .values
            .iter()
            .map(|(name, raw)| {
                let kind = fields
                    .iter()
                    .find(|field| &field.name == name)
                    .expect("a field of the parser that read the title")
                    .kind;

                (name.clone(), kind.normalize(raw))
            })
            .collect();

        Some((reading.parser, values))
    }
}
