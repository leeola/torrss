//! Which copy of one release the reader wants.
//!
//! A search decides whether a release is wanted at all. A preference decides
//! between the copies of one that is. Two qualities of one episode are one
//! identity, and only one of them is worth taking.
//!
//! A preference is one ordered list of values for one parser field name,
//! best first. Keying by field name rather than by search means a reader
//! states once that 2160p beats 1080p, and every search reading that field
//! follows it.
//!
//! A value the list does not name is ranked below every value it does, so a
//! short list is a statement about what the reader wants rather than a
//! filter that hides the rest.

// FIXME: Nothing outside the tests reads a preference yet. The ranking that
// orders one identity's copies is the caller this waits on.
#![allow(dead_code)]

pub(crate) mod store;

use std::collections::BTreeMap;

/// The ordered lists a reader has stated, by field name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Preferences {
    lists: BTreeMap<String, Vec<String>>,
}

impl Preferences {
    pub(crate) fn new(lists: BTreeMap<String, Vec<String>>) -> Self {
        Self { lists }
    }

    /// The values preferred for `field`, best first, or an empty slice when
    /// the reader has stated nothing about it.
    pub(crate) fn values(&self, field: &str) -> &[String] {
        self.lists.get(field).map_or(&[], Vec::as_slice)
    }

    /// Every field the reader has stated a list for, in name order.
    pub(crate) fn fields(&self) -> impl Iterator<Item = &str> {
        self.lists.keys().map(String::as_str)
    }
}
