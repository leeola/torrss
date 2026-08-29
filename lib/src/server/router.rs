use std::sync::Arc;
use std::{io, path::Path};

use topcoat::{
    asset::{AssetBundle, RouterBuilderAssetExt},
    router::{Router, RouterBuilderDiscoverExt},
};

use crate::feed::registry::FeedRegistry;
use crate::server::state::RulesetSwitches;
use crate::server::trace::RequestSpan;
use crate::services::Services;
use crate::torrent::sync::SyncState;

/// Builds the router from every discovered page, layout, and layer.
///
/// Discovery collects the annotated functions across the whole binary at link
/// time, so a route appears here by existing rather than by being listed.
///
/// `services`, `registry`, and `sync` each reach a handler through the app
/// context, keyed by their type. The registry and the sync state arrive already
/// shared, because the two poll tasks hold the same ones.
pub(super) fn build(
    assets: Option<&Path>,
    services: Services,
    registry: Arc<FeedRegistry>,
    sync: Arc<SyncState>,
) -> io::Result<Router> {
    Ok(Router::builder()
        .discover()
        .assets(load_assets(assets)?)
        .app_context(RulesetSwitches::new())
        .app_context(services)
        .app_context(registry)
        .app_context(sync)
        .layer(RequestSpan)
        .build())
}

/// Reads the bundle from `dir`, or from beside the executable for [`None`].
///
/// [`AssetBundle::load_dir`] reports a bare `NotFound` for a directory holding
/// no manifest, which reads as an unrelated failure when the caller named the
/// directory. The path goes into the message to separate the two.
fn load_assets(dir: Option<&Path>) -> io::Result<AssetBundle> {
    let Some(dir) = dir else {
        return AssetBundle::load();
    };

    AssetBundle::load_dir(dir).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("asset bundle at {}: {error}", dir.display()),
        )
    })
}
