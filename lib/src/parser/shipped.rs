//! The parsers the binary carries.
//!
//! A fresh database stores none, so these are what read a title until the
//! reader writes one of their own. They are declared in the order the engine
//! tries them, most specific first, because
//! [`Engine::read`](crate::engine::Engine::read) takes the first parser that
//! reads a name.
//!
//! Each names its fields as [`super::PRESETS`] does. An import and a search
//! therefore find `show`, `season`, `episodeNumber`, and `resolution` under
//! one name, whichever parser read them.
//!
//! A separator is `[. _]`. A tracker writes a name in dots, and a client
//! writes the folder it unpacked into in spaces. The resolution admits `(`
//! as well, because a tracker that brackets its quality tags opens the run
//! with it.

use super::FieldKind::{Enum, Episode, Number, Season, Text};
use super::{Field, FieldKind, Parser};

/// The parsers the binary carries, in the order the engine tries them.
///
/// None carries a title test. Those belong to the reader who writes a parser
/// and names the titles it must read, where the `READINGS` table in this
/// module's tests checks what a shipped one reads.
pub(crate) fn parsers() -> Vec<Parser> {
    // The episode name is a lazy run that nothing inside it ends, so the
    // resolution behind it is what stops it.
    let series_tags = tags().into_iter().map(|tag| Field {
        required: tag.name == "resolution",
        ..tag
    });

    vec![
        // This leads the set because a required bracketed group tag is the
        // most specific opening any of these has. A scene name never starts
        // with one, so nothing else loses a title to it. A name such as
        // [OpenReel] Coastal Ecology S2 - 05v2 carries an S2 token, and the
        // series parsers read that token as their own season.
        //
        // The identity fields match series, so a show numbered by season
        // files with its scene releases. One numbered without a season keeps
        // an empty season part, which is what the positional key is for.
        //
        // A v2 suffix marks a re-release of the same episode, so it stays out
        // of the number. Only the extension comes from tags(), because this
        // shape writes its quality inside brackets rather than as trailing
        // dotted tags.
        Parser {
            id: "anime".to_owned(),
            name: "Anime".to_owned(),
            fields: vec![
                field(
                    "publisher",
                    Text,
                    Some(r"^\[(?<publisher>[^\]]+)\]"),
                    true,
                    false,
                    false,
                ),
                field("show", Text, Some(r"\s(?<show>.+?)"), true, true, true),
                field(
                    "season",
                    Season,
                    Some(r"\sS(?<season>\d{1,2})"),
                    false,
                    true,
                    true,
                ),
                field(
                    "episodeNumber",
                    Episode,
                    Some(r"\s-\s(?<episodeNumber>\d{1,4})(?:v\d)?"),
                    true,
                    true,
                    false,
                ),
                field(
                    "resolution",
                    Enum,
                    Some(r"(?i)[\[(](?:[^\])]*\s)?(?<resolution>480p|720p|1080p|2160p)"),
                    false,
                    false,
                    false,
                ),
                field(
                    "checksum",
                    Text,
                    Some(r"\[(?<checksum>[0-9A-Fa-f]{8})\]"),
                    false,
                    false,
                    false,
                ),
            ]
            .into_iter()
            .chain(tags().into_iter().filter(|tag| tag.name == "extension"))
            .collect(),
            tests: Vec::new(),
            built_in: true,
        },
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
        // The three date parts are the identity, so an episode of this shape
        // files into show+year+month+day. A talk show names no season, and
        // the date is what tells one broadcast from the next.
        //
        // This entry precedes the film parser, because a dated title carries
        // a year too and the film parser reads a title followed by one.
        Parser {
            id: "series-daily".to_owned(),
            name: "Series by date".to_owned(),
            fields: vec![
                field("show", Text, Some(r"^(?<show>.+?)"), true, true, true),
                field(
                    "year",
                    Number,
                    Some(r"[. _](?<year>(?:19|20)\d{2})"),
                    true,
                    true,
                    true,
                ),
                field(
                    "month",
                    Number,
                    Some(r"[. _](?<month>0[1-9]|1[0-2])"),
                    true,
                    true,
                    true,
                ),
                field(
                    "day",
                    Number,
                    Some(r"[. _](?<day>0[1-9]|[12]\d|3[01])"),
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
        // The movie run is greedy, so the last year in the title is the one
        // read. That keeps a year written inside a name, such as
        // The.2000.Drift, out of the year field.
        //
        // The year also matches its parenthesized spelling, because a client
        // names the folder it unpacked into Title (2024).
        Parser {
            id: "film".to_owned(),
            name: "Film".to_owned(),
            fields: vec![
                field("movie", Text, Some(r"^(?<movie>.+)"), true, true, true),
                field(
                    "year",
                    Number,
                    Some(r"[. _(](?<year>(?:19|20)\d{2})\)?"),
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
        // A tracker that writes its tags inside parentheses opens the run
        // with the resolution, so `(` separates it as a dot does.
        field(
            "resolution",
            Enum,
            Some(r"(?i)[. _(](?<resolution>480p|576p|720p|1080p|1080i|2160p)"),
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
    use crate::engine::Engine;
    use crate::search::Search;

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
            "[OpenReel] Coastal Ecology - 18 [1080p][A1B2C3D4].mkv",
            Some((
                "anime",
                &[
                    ("publisher", "openreel"),
                    ("show", "coastal ecology"),
                    ("episodeNumber", "18"),
                    ("resolution", "1080p"),
                    ("checksum", "a1b2c3d4"),
                    ("extension", "mkv"),
                ],
            )),
        ),
        (
            "[OpenReel] Coastal Ecology S2 - 05v2 (720p).mkv",
            Some((
                "anime",
                &[
                    ("publisher", "openreel"),
                    ("show", "coastal ecology"),
                    ("season", "2"),
                    ("episodeNumber", "5"),
                    ("resolution", "720p"),
                    ("extension", "mkv"),
                ],
            )),
        ),
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
            "Coastal Ecology S02 (2160p WEB-DL H265 DDP Atmos 5.1 English - OpenReel)",
            Some((
                "series-loose",
                &[
                    ("show", "coastal ecology"),
                    ("season", "2"),
                    ("resolution", "2160p"),
                    ("source", "web dl"),
                    ("codec", "h265"),
                    ("audio", "atmos"),
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
        (
            "Coastal.Ecology.2024.03.15.1080p.WEB-DL-OpenReel",
            Some((
                "series-daily",
                &[
                    ("show", "coastal ecology"),
                    ("year", "2024"),
                    ("month", "3"),
                    ("day", "15"),
                    ("resolution", "1080p"),
                    ("source", "web dl"),
                    ("publisher", "openreel"),
                ],
            )),
        ),
        (
            "Coastal.Drift.2024.1080p.BluRay.x264.DTS-HD-MeridianPress.mkv",
            Some((
                "film",
                &[
                    ("movie", "coastal drift"),
                    ("year", "2024"),
                    ("resolution", "1080p"),
                    ("source", "bluray"),
                    ("codec", "x264"),
                    ("audio", "dts hd"),
                    ("publisher", "meridianpress"),
                    ("extension", "mkv"),
                ],
            )),
        ),
        (
            "Coastal Drift (2024) 2160p WEB-DL",
            Some((
                "film",
                &[
                    ("movie", "coastal drift"),
                    ("year", "2024"),
                    ("resolution", "2160p"),
                    ("source", "web dl"),
                ],
            )),
        ),
        (
            "The.2000.Drift.2024.720p",
            Some((
                "film",
                &[
                    ("movie", "the 2000 drift"),
                    ("year", "2024"),
                    ("resolution", "720p"),
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

    /// A search on `parser` that writes no condition, so it claims every
    /// title the parser reads.
    fn claiming(id: &str, parser: &str) -> Search {
        Search {
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
    fn a_shipped_parser_carries_no_title_tests() {
        assert!(
            parsers().iter().all(|parser| parser.tests.is_empty()),
            "the titles a shipped parser must read are checked by READINGS"
        );
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
