//! Query-string helpers for the id lists the pages carry.
//!
//! The list is what the grab procedure receives and what the listing shard
//! reads a row's checked state from. It is comma-separated because the
//! browser builds it from a `Set` with `join`.

/// A comma-separated id list held in one value.
#[derive(Clone, Copy)]
pub(super) struct IdList<'a>(&'a str);

impl<'a> IdList<'a> {
    pub(super) fn new(raw: Option<&'a str>) -> Self {
        Self(raw.unwrap_or_default())
    }

    /// The ids collected, for a caller that tests membership many times.
    pub(super) fn entries(self) -> Vec<&'a str> {
        self.split().collect()
    }

    pub(super) fn contains(self, id: &str) -> bool {
        self.split().any(|entry| entry == id)
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
