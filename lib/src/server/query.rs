//! Query-string helpers for the id lists the pages carry in the URL.
//!
//! Selecting a feed result, pinning a candidate, and replacing an inherited
//! field all live in the query string rather than in browser state. That
//! keeps a view shareable, and it survives the full page load that every
//! control triggers.

/// A comma-separated id list held in one query parameter.
#[derive(Clone, Copy)]
pub(super) struct IdList<'a>(&'a str);

impl<'a> IdList<'a> {
    pub(super) fn new(raw: Option<&'a str>) -> Self {
        Self(raw.unwrap_or_default())
    }

    /// The raw parameter value, ready to write back into a URL.
    pub(super) fn as_str(self) -> &'a str {
        self.0
    }

    /// The ids collected, for a caller that tests membership many times.
    pub(super) fn entries(self) -> Vec<&'a str> {
        self.split().collect()
    }

    pub(super) fn contains(self, id: &str) -> bool {
        self.split().any(|entry| entry == id)
    }

    pub(super) fn len(self) -> usize {
        self.split().count()
    }

    pub(super) fn is_empty(self) -> bool {
        self.len() == 0
    }

    /// The list with `id` removed when present, and appended otherwise.
    pub(super) fn toggled(self, id: &str) -> String {
        if self.contains(id) {
            return self
                .split()
                .filter(|entry| *entry != id)
                .collect::<Vec<_>>()
                .join(",");
        }

        match self.0 {
            "" => id.to_owned(),
            list => format!("{list},{id}"),
        }
    }

    fn split(self) -> impl Iterator<Item = &'a str> {
        self.0.split(',').filter(|entry| !entry.is_empty())
    }
}

/// Builds a URL from `path`, the query keys holding a value, and an anchor.
///
/// A key with an empty value is dropped, so a cleared selection leaves no
/// stray parameter behind.
///
/// Pass an `anchor` on any control that sits below the fold. A query-only
/// change is a fresh navigation that lands the reader at the top of the
/// document, far from the list they came from.
pub(super) fn url(path: &str, keys: &[(&str, &str)], anchor: &str) -> String {
    let query = keys
        .iter()
        .filter(|(_, value)| !value.is_empty())
        .map(|(key, value)| format!("{key}={}", encode(value)))
        .collect::<Vec<_>>()
        .join("&");

    let separator = if query.is_empty() { "" } else { "?" };

    format!("{path}{separator}{query}{anchor}")
}

/// Percent-encodes a query value.
///
/// A value that carries a whole URL, such as the page a switch returns to,
/// holds `?`, `&`, and `#`. Left raw, those characters end the value and the
/// rest of it parses as separate keys.
///
/// `,` `:` and `/` stay literal. They are legal inside a query value, and
/// keeping them readable makes a shared link easy to check by eye.
fn encode(value: &str) -> String {
    value
        .chars()
        .map(|character| match character {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '.' | '_' | '~' | ',' | ':' | '/' => {
                character.to_string()
            }
            _ => {
                let mut encoded = String::new();
                let mut buffer = [0_u8; 4];
                for byte in character.encode_utf8(&mut buffer).as_bytes() {
                    encoded.push_str(&format!("%{byte:02X}"));
                }
                encoded
            }
        })
        .collect()
}
