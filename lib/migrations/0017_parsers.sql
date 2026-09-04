-- The parsers a reader writes, and what each one reads a filename apart by.
--
-- A parser composes its fields in order into the one regex that cuts a name
-- into the values behind it. It claims no title and filters no feed, so it
-- carries no enabled column: a switch on a parser would turn nothing off.
--
-- The fields, the saved tests, and the expectations all belong to their
-- parser and leave with it, so every table below cascades.
--
-- `position` orders the fields as the regex composes them and the tests as
-- the editor shows them. It is contiguous from zero, because a save rewrites
-- the whole list.
CREATE TABLE parsers (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL
);

CREATE TABLE parser_fields (
    parser TEXT NOT NULL REFERENCES parsers(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    pattern TEXT,
    required INTEGER NOT NULL,
    identity INTEGER NOT NULL,
    tight INTEGER NOT NULL,
    PRIMARY KEY (parser, position)
);

CREATE TABLE parser_tests (
    parser TEXT NOT NULL REFERENCES parsers(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    title TEXT NOT NULL,
    PRIMARY KEY (parser, position)
);

CREATE TABLE parser_test_values (
    parser TEXT NOT NULL,
    position INTEGER NOT NULL,
    field TEXT NOT NULL,
    expected TEXT NOT NULL,
    PRIMARY KEY (parser, position, field),
    FOREIGN KEY (parser, position) REFERENCES parser_tests(parser, position) ON DELETE CASCADE
);
