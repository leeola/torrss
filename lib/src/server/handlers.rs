use topcoat::{
    Result,
    context::Cx,
    context::app_context,
    router::{
        error::{RouterErrorExt, redirect},
        page, path_param, query_params, route,
    },
    view::view,
};

use crate::{
    mock::{self, Diff, RULESETS},
    server::{
        components,
        query::{self, IdList},
        state::RulesetSwitches,
    },
};

path_param!(ruleset_id);

/// Controls which feed results show and which of them are selected.
#[query_params(error = bad_request)]
struct FeedView {
    /// [`mock::Ruleset::id`] of the only ruleset to list, or absent for all.
    ruleset: Option<String>,

    /// Comma-separated [`mock::Release::id`] values marked to grab.
    selected: Option<String>,
}

impl FeedView {
    fn active(&self) -> Option<&str> {
        self.ruleset.as_deref().filter(|id| !id.is_empty())
    }

    fn selection(&self) -> IdList<'_> {
        IdList::new(self.selected.as_deref())
    }
}

/// Builds a feed URL carrying the ruleset filter and the selection.
fn feed_url(ruleset: Option<&str>, selected: &str, anchor: &str) -> String {
    query::url(
        "/",
        &[
            ("ruleset", ruleset.unwrap_or_default()),
            ("selected", selected),
        ],
        anchor,
    )
}

#[page("/")]
async fn feed(cx: &Cx) -> Result {
    let view = query_params::<FeedView>(cx)?;
    let active = view.active();
    let selection = view.selection();

    // A disabled ruleset filters nothing, so its releases never reach the feed.
    let switches = app_context::<RulesetSwitches>(cx);
    let releases: Vec<_> = mock::releases(active)
        .filter(|release| switches.is_enabled(release.ruleset))
        .collect();

    // Selecting every listed release is one link, so the target is the whole
    // listed set rather than a toggle of what is already selected.
    let all_listed = releases
        .iter()
        .map(|release| release.id)
        .collect::<Vec<_>>()
        .join(",");

    let selected_here = releases
        .iter()
        .filter(|release| selection.contains(release.id))
        .count();

    view! {
        <h1 class="text-2xl font-semibold tracking-tight">"Feed results"</h1>
        <p class="mt-1 text-sm text-slate-400">
            (releases.len()) " releases matched by the enabled rulesets."
            let disabled = switches.disabled_count();
            if disabled > 0 {
                " "
                <a
                    href="/admin"
                    class="underline decoration-slate-700 underline-offset-2 hover:text-slate-200"
                >
                    (disabled) " disabled rulesets are filtering nothing."
                </a>
            }
        </p>

        <nav class="mt-6 flex flex-wrap gap-2">
            components::filter_chip(
                href: feed_url(None, selection.as_str(), "#results"),
                label: "All",
                current: active.is_none(),
            )
            for ruleset in RULESETS.iter().filter(|ruleset| switches.is_enabled(ruleset.id)) {
                components::filter_chip(
                    href: feed_url(Some(ruleset.id), selection.as_str(), "#results"),
                    label: ruleset.name,
                    current: active == Some(ruleset.id),
                )
            }
        </nav>

        <div
            id="results"
            class="mt-6 flex scroll-mt-24 flex-wrap items-center justify-between gap-3 rounded-lg border border-slate-800 bg-slate-900/40 px-4 py-3"
        >
            <div class="flex flex-wrap items-center gap-3">
                components::checkbox(
                    href: feed_url(
                        active,
                        if selected_here == releases.len() { "" } else { all_listed.as_str() },
                        "#results",
                    ),
                    checked: selected_here == releases.len() && !releases.is_empty(),
                    label: "Select every listed release",
                )
                <span class="text-sm text-slate-300">
                    if selection.is_empty() {
                        "Nothing selected"
                    } else {
                        (selection.len()) " selected"
                    }
                </span>
                if !selection.is_empty() {
                    <a
                        href=(feed_url(active, "", "#results"))
                        class="text-xs text-slate-500 underline decoration-slate-700 underline-offset-2 hover:text-slate-300"
                    >
                        "Clear"
                    </a>
                }
            </div>

            <button
                type="button"
                disabled=(selection.is_empty())
                class="rounded-md bg-sky-400 px-3 py-1.5 text-sm font-medium text-slate-900 hover:bg-sky-300 disabled:cursor-not-allowed disabled:bg-slate-800 disabled:text-slate-500"
            >
                if selection.is_empty() {
                    "Grab selected"
                } else {
                    "Grab " (selection.len()) " releases"
                }
            </button>
        </div>

        if releases.is_empty() {
            <p class="mt-4 rounded-lg border border-slate-800 px-4 py-8 text-center text-sm text-slate-500">
                "No release matched this ruleset."
            </p>
        } else {
            <ul class="mt-4 flex flex-col gap-2">
                for release in releases {
                    components::release_row(
                        release: release,
                        toggle_href: feed_url(
                            active,
                            &selection.toggled(release.id),
                            &format!("#release-{}", release.id),
                        ),
                        selected: selection.contains(release.id),
                    )
                }
            </ul>
        }
    }
}

/// Where a switch returns the reader after it flips a ruleset.
#[query_params(error = bad_request)]
struct SwitchReturn {
    back: Option<String>,
}

/// Flips one ruleset between enabled and disabled, then returns to `back`.
///
/// This posts rather than links because it writes shared state. It redirects
/// instead of rendering, so a reload never repeats the flip.
#[route(POST "/admin/rulesets/{ruleset_id}/enabled")]
async fn set_enabled(cx: &Cx) -> Result<&'static str> {
    let ruleset = mock::ruleset(path_param::<RulesetId>(cx)).ok_or_not_found()?;
    let back = query_params::<SwitchReturn>(cx)?
        .back
        .clone()
        .unwrap_or_else(|| "/admin".to_owned());

    app_context::<RulesetSwitches>(cx).toggle(ruleset.id);

    Err(redirect(back).into())
}

/// Builds the action a ruleset's switch posts to.
fn switch_action(ruleset: &str, back: &str) -> String {
    query::url(
        &format!("/admin/rulesets/{ruleset}/enabled"),
        &[("back", back)],
        "",
    )
}

#[page("/admin")]
async fn admin(cx: &Cx) -> Result {
    let switches = app_context::<RulesetSwitches>(cx);

    view! {
        <div class="flex flex-wrap items-end justify-between gap-4">
            <div>
                <h1 class="text-2xl font-semibold tracking-tight">"Rulesets"</h1>
                <p class="mt-1 text-sm text-slate-400">
                    "A ruleset decides which filenames it claims and which parts it pulls out of
                    them. A disabled ruleset filters nothing, so its releases stay out of the feed."
                </p>
            </div>
            <a
                href="/admin/rulesets/new"
                class="rounded-md bg-slate-100 px-3 py-1.5 text-sm font-medium text-slate-900 hover:bg-white"
            >
                "New ruleset"
            </a>
        </div>

        <ul id="rulesets" class="mt-6 flex scroll-mt-24 flex-col gap-3">
            for base in RULESETS.iter().filter(|ruleset| ruleset.inherits.is_none()) {
                components::ruleset_card(
                    ruleset: base,
                    nested: false,
                    enabled: switches.is_enabled(base.id),
                )

                for child in base.children() {
                    components::ruleset_card(
                        ruleset: child,
                        nested: true,
                        enabled: switches.is_enabled(child.id),
                    )
                }
            }
        </ul>
    }
}

/// Controls the candidate list under the editor.
///
/// Every key rides in the URL rather than in browser state, so a reviewer
/// shares an exact view and keeps it across the reload that follows a save.
#[query_params(error = bad_request)]
struct MatchView {
    /// [`Diff::slug`] of the only state to list, or absent for every state.
    diff: Option<String>,

    /// Comma-separated [`Candidate::id`] values held at the top of the list.
    pinned: Option<String>,

    /// Comma-separated [`mock::Field::name`] values flipped between inherited
    /// and replaced since the last save.
    replaced: Option<String>,
}

impl MatchView {
    fn filter(&self) -> Option<Diff> {
        Diff::from_slug(self.diff.as_deref()?)
    }

    /// The whole query state, ready to rebuild any link on the page.
    fn query(&self) -> EditorQuery<'_> {
        EditorQuery {
            diff: self.filter(),
            pinned: IdList::new(self.pinned.as_deref()),
            replaced: IdList::new(self.replaced.as_deref()),
        }
    }
}

/// Every key the editor carries in its URL.
///
/// A control rebuilds the whole struct with one field changed rather than
/// editing a single query key. Changing the filter therefore never drops the
/// pins, and replacing a field never drops the filter.
#[derive(Clone, Copy)]
struct EditorQuery<'a> {
    diff: Option<Diff>,
    pinned: IdList<'a>,
    replaced: IdList<'a>,
}

impl EditorQuery<'_> {
    fn url(&self, ruleset: &str, anchor: &str) -> String {
        self.build(
            ruleset,
            anchor,
            self.pinned.as_str(),
            self.replaced.as_str(),
        )
    }

    /// The same query with the pin list replaced.
    fn with_pins(&self, pins: &str, ruleset: &str, anchor: &str) -> String {
        self.build(ruleset, anchor, pins, self.replaced.as_str())
    }

    /// The same query with the replaced-field list replaced.
    fn with_replaced(&self, replaced: &str, ruleset: &str, anchor: &str) -> String {
        self.build(ruleset, anchor, self.pinned.as_str(), replaced)
    }

    fn build(&self, ruleset: &str, anchor: &str, pinned: &str, replaced: &str) -> String {
        query::url(
            &format!("/admin/rulesets/{ruleset}"),
            &[
                ("diff", self.diff.map_or("", Diff::slug)),
                ("pinned", pinned),
                ("replaced", replaced),
            ],
            anchor,
        )
    }
}

#[page("/admin/rulesets/{ruleset_id}")]
async fn ruleset_editor(cx: &Cx) -> Result {
    let ruleset = mock::ruleset(path_param::<RulesetId>(cx)).ok_or_not_found()?;
    let view = query_params::<MatchView>(cx)?;
    let q = view.query();
    let filter = q.diff;

    // A pinned candidate stays visible under every filter. Pinning exists to
    // keep one filename in sight while the patterns above it change.
    let (pinned, rest): (Vec<_>, Vec<_>) = ruleset
        .candidates
        .iter()
        .partition(|candidate| q.pinned.contains(candidate.id));

    let listed: Vec<_> = rest
        .into_iter()
        .filter(|candidate| filter.is_none_or(|state| candidate.diff == state))
        .collect();

    let replacements = q.replaced.entries();
    let fields = ruleset.resolved_fields(&replacements);
    let inheriting = ruleset.parent().is_some();
    let enabled = app_context::<RulesetSwitches>(cx).is_enabled(ruleset.id);

    view! {
        <nav class="text-sm text-slate-500">
            <a href="/admin" class="hover:text-slate-300">"Rulesets"</a>
            " / "
            <span class="text-slate-300">(ruleset.name)</span>
        </nav>

        <div id="top" class="mt-3 flex scroll-mt-24 flex-wrap items-start justify-between gap-4">
            <div>
                <div class="flex flex-wrap items-center gap-3">
                    <h1 class="text-2xl font-semibold tracking-tight">(ruleset.name)</h1>
                    components::status_badge(enabled: enabled)
                </div>
                <p class="mt-1 text-sm text-slate-400">(ruleset.summary)</p>
            </div>

            <div class="flex flex-wrap items-center gap-2">
                <button
                    type="button"
                    class="rounded-md border border-slate-700 px-3 py-1.5 text-sm text-slate-300 hover:border-slate-600 hover:text-slate-100"
                >
                    "Add field"
                </button>
                <button
                    type="submit"
                    form="ruleset-fields"
                    class="rounded-md bg-slate-100 px-3 py-1.5 text-sm font-medium text-slate-900 hover:bg-white"
                >
                    "Save ruleset"
                </button>
                components::status_toggle(
                    enabled: enabled,
                    action: switch_action(ruleset.id, &q.url(ruleset.id, "#top")),
                )
            </div>
        </div>

        match ruleset.parent() {
            Some(parent) => <p class="mt-3 rounded-md border border-slate-800 bg-slate-900/40 px-3 py-2 text-xs text-slate-400">
                "Narrows "
                <a
                    href=(format!("/admin/rulesets/{}", parent.id))
                    class="text-slate-200 underline decoration-slate-700 underline-offset-2 hover:text-white"
                >(parent.name)</a>
                ". A greyed field carries the parent's value. Replace one to give this ruleset its own."
            </p>,
            None => "",
        }

        <section class="mt-6 rounded-lg border border-slate-800 bg-slate-900/30 px-4 py-4">
            <h2 class="text-xs font-medium tracking-wide text-slate-500 uppercase">"Preview"</h2>
            <div class="mt-2">
                components::filename(segments: ruleset.sample, ruleset: ruleset.id)
            </div>
        </section>

        <form id="ruleset-fields" class="mt-6 rounded-lg border border-slate-800 bg-slate-900/40">
            <div class="flex flex-wrap items-center justify-between gap-3 px-4 py-3">
                <h2 class="text-sm font-semibold text-slate-100">
                    "Fields " <span class="text-slate-500">"(" (fields.len()) ")"</span>
                </h2>
                <p class="text-xs text-slate-500">
                    if inheriting {
                        "A greyed field is inherited. Use the source column to replace it."
                    } else {
                        "Each field names a capture group and the type its value parses as."
                    }
                </p>
            </div>

            for resolved in fields {
                components::field_row(
                    resolved: resolved,
                    inheriting: inheriting,
                    toggle_href: q.with_replaced(
                            &q.replaced.toggled(resolved.field.name),
                            ruleset.id,
                            &format!("#field-{}", resolved.field.part.slug()),
                        ),
                )
            }

        </form>

        <section id="matches" class="mt-8 scroll-mt-24">
            <div class="flex flex-wrap items-end justify-between gap-3">
                <div>
                    <h2 class="text-lg font-semibold tracking-tight">"Matches"</h2>
                    <p class="mt-1 text-sm text-slate-400">
                        (ruleset.candidates.len())
                        " candidates from " (ruleset.feeds.join(", ")) ", against the edited rules."
                    </p>
                </div>
                <p class="text-xs text-slate-500">
                    (ruleset.diff_count(Diff::Added)) " gained, "
                    (ruleset.diff_count(Diff::Removed)) " lost"
                </p>
            </div>

            <nav class="mt-4 flex flex-wrap gap-2">
                components::diff_filter(
                    href: EditorQuery { diff: None, ..q }.url(ruleset.id, "#matches"),
                    label: "All",
                    count: ruleset.candidates.len(),
                    current: filter.is_none(),
                )
                for state in Diff::ALL {
                    components::diff_filter(
                        href: EditorQuery { diff: Some(*state), ..q }.url(ruleset.id, "#matches"),
                        label: state.label(),
                        count: ruleset.diff_count(*state),
                        current: filter == Some(*state),
                    )
                }
            </nav>

            if !pinned.is_empty() {
                <div class="mt-4 rounded-lg border border-amber-400/25 bg-amber-400/5 px-3 py-3">
                    <h3 class="text-xs font-medium tracking-wide text-amber-300/80 uppercase">
                        "Pinned"
                    </h3>
                    <ul class="mt-2 flex flex-col gap-2">
                        for candidate in pinned {
                            components::candidate_row(
                                candidate: candidate,
                                ruleset: ruleset.id,
                                pin_href: q.with_pins(
                                        &q.pinned.toggled(candidate.id),
                                        ruleset.id,
                                        &format!("#match-{}", candidate.id),
                                        ),
                                pinned: true,
                            )
                        }
                    </ul>
                </div>
            }

            if listed.is_empty() {
                <p class="mt-4 rounded-lg border border-slate-800 px-4 py-8 text-center text-sm text-slate-500">
                    "No candidate sits in this state."
                </p>
            } else {
                <ul class="mt-4 flex flex-col gap-2">
                    for candidate in listed {
                        components::candidate_row(
                            candidate: candidate,
                            ruleset: ruleset.id,
                            pin_href: q.with_pins(
                                    &q.pinned.toggled(candidate.id),
                                    ruleset.id,
                                    &format!("#match-{}", candidate.id),
                                    ),
                            pinned: false,
                        )
                    }
                </ul>
            }
        </section>
    }
}

#[page("/admin/rulesets/new")]
async fn new_ruleset() -> Result {
    view! {
        <nav class="text-sm text-slate-500">
            <a href="/admin" class="hover:text-slate-300">"Rulesets"</a>
            " / "
            <span class="text-slate-300">"New"</span>
        </nav>

        <h1 class="mt-3 text-2xl font-semibold tracking-tight">"New ruleset"</h1>
        <p class="mt-1 text-sm text-slate-400">
            "Start from nothing, or narrow an existing ruleset to one series."
        </p>

        <form class="mt-6 flex flex-col gap-4 rounded-lg border border-slate-800 bg-slate-900/40 px-4 py-4">
            <div>
                <label for="name" class="block text-xs text-slate-500">"Name"</label>
                <input
                    id="name"
                    type="text"
                    name="name"
                    placeholder="Coastal Ecology"
                    class="mt-1 w-full rounded-md border border-slate-800 bg-slate-950 px-2 py-1.5 text-sm text-slate-100 focus:border-slate-600 focus:outline-none"
                >
            </div>

            <div>
                <label for="summary" class="block text-xs text-slate-500">"Summary"</label>
                <input
                    id="summary"
                    type="text"
                    name="summary"
                    placeholder="Narrows the episode rules to one series."
                    class="mt-1 w-full rounded-md border border-slate-800 bg-slate-950 px-2 py-1.5 text-sm text-slate-100 focus:border-slate-600 focus:outline-none"
                >
            </div>

            <div>
                <label for="inherits" class="block text-xs text-slate-500">"Inherit from"</label>
                <select
                    id="inherits"
                    name="inherits"
                    class="mt-1 w-full rounded-md border border-slate-800 bg-slate-950 px-2 py-1.5 text-sm text-slate-100 focus:border-slate-600 focus:outline-none"
                >
                    <option value="">"Nothing. Declare every field here."</option>
                    for base in RULESETS.iter().filter(|ruleset| ruleset.inherits.is_none()) {
                        <option value=(base.id)>
                            (base.name) " (" (base.fields.len()) " fields)"
                        </option>
                    }
                </select>
                <p class="mt-2 text-xs text-slate-500">
                    "An inherited ruleset starts with every field greyed out. Replace only the
                    field that names the series, and the other rules keep tracking the parent."
                </p>
            </div>

            <div class="flex flex-wrap items-center gap-3 border-t border-slate-800 pt-4">
                <button
                    type="submit"
                    class="rounded-md bg-slate-100 px-3 py-1.5 text-sm font-medium text-slate-900 hover:bg-white"
                >
                    "Create ruleset"
                </button>
                <a href="/admin" class="text-sm text-slate-400 hover:text-slate-200">"Cancel"</a>
            </div>
        </form>
    }
}
