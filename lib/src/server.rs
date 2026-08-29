//! The HTTP application: its routes, its rendered pages, and its listener.
//!
//! Nothing outside this module calls the functions in `handlers` and
//! `templates`. Their `#[page]` and `#[layout]` attributes register each
//! function with the router at link time, so those modules take effect by
//! being compiled in rather than by being called.

mod components;
mod format;
mod handlers;
mod query;
mod router;
mod serve;
mod state;
mod templates;

pub use serve::{Config, DEFAULT_HOST, DEFAULT_PORT, serve};
