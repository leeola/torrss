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
