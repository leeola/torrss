use std::{io, path::Path};

use topcoat::{
    asset::{AssetBundle, RouterBuilderAssetExt},
    router::{Router, RouterBuilderDiscoverExt},
};

use crate::server::state::RulesetSwitches;
use crate::services::Services;

/// Builds the router from every discovered page, layout, and layer.
///
/// Discovery collects the annotated functions across the whole binary at link
/// time, so a route appears here by existing rather than by being listed.
///
/// `services` reaches a handler through the app context, keyed by its type.
pub(super) fn build(assets: Option<&Path>, services: Services) -> io::Result<Router> {
    Ok(Router::builder()
        .discover()
        .assets(load_assets(assets)?)
        .app_context(RulesetSwitches::new())
        .app_context(services)
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
