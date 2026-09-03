use topcoat::{
    Result,
    view::{class, component, view},
};

use super::format;
use super::listing::ParsedValue;
use super::matches::Match;
use crate::feed::registry::FeedCheck;
use crate::rules::Engine;
use crate::ruleset::{FieldKind, FieldSource, Part, ResolvedField, Ruleset, Segment};
use crate::store::StoredItem;

/// Renders a filename with every claimed run tinted by its part.
///
/// Each claimed run links to the field that produced it, anchored inside
/// `ruleset`'s editor, so a reader jumps from a value to the rule behind it.
#[component]
pub(crate) async fn filename(segments: &[Segment<'_>], ruleset: &str) -> Result {
    view! {
        <span class="font-mono text-sm break-all">
            for segment in segments {
                match segment.part {
                    Some(part) => <a
                        href=(format!("/admin/rulesets/{ruleset}#field-{}", part.slug()))
                        class=(class!(
                            "rounded-sm px-0.5 py-px hover:underline hover:decoration-dotted",
                            part.classes(),
                        ))
                        title=(format!("edit the {} rule", part.label()))
                    >(segment.text)</a>,
                    None => <span class="text-slate-500">(segment.text)</span>,
                }
            }
        </span>
    }
}

/// One feed filter, linking to the feed narrowed to a single ruleset.
#[component]
pub(crate) async fn filter_chip(#[into] href: String, label: &str, current: bool) -> Result {
    view! {
        <a
            href=(href)
            class=(class!(
                "rounded-full border px-3 py-1 text-xs transition-colors",
                "border-slate-600 bg-slate-800 text-slate-100" if current
                    else "border-slate-800 text-slate-400 hover:border-slate-700 hover:text-slate-200",
            ))
        >
            (label)
        </a>
    }
}

/// One ruleset that claims a listed title.
///
/// A row links a claimant by id and shows its name, and it holds nothing
/// else of the ruleset. Borrowing the whole ruleset would tie every row to
/// the engine that resolved it, which outlives no request.
pub(crate) struct Claimant {
    pub id: String,
    pub name: String,
}

/// One entry in the diff filter bar, carrying its own count.
#[component]
pub(crate) async fn diff_filter(
    #[into] href: String,
    label: &str,
    count: usize,
    current: bool,
) -> Result {
    view! {
        <a
            href=(href)
            class=(class!(
                "rounded-full border px-3 py-1 text-xs transition-colors",
                "border-slate-600 bg-slate-800 text-slate-100" if current
                    else "border-slate-800 text-slate-400 hover:border-slate-700 hover:text-slate-200",
            ))
            if current {
                aria-current="true"
            }
        >
            (label) " " <span class="text-slate-500">(count)</span>
        </a>
    }
}

/// One stored title in the editor, tinted by how the edit changed it.
///
/// The pin control is a link rather than a button because pinning lives in
/// the query string. That keeps the pinned set shareable and survives the
/// reload that follows every pattern edit.
#[component]
pub(crate) async fn match_row(
    matched: &Match<'_>,
    ruleset: &str,
    #[into] pin_href: String,
    pinned: bool,
) -> Result {
    view! {
        <li
            id=(format!("match-{}", matched.id))
            class=(class!(
                "flex items-start gap-3 rounded-lg border px-3 py-2.5 scroll-mt-24",
                matched.diff.row_classes(),
            ))
        >
            <a
                href=(pin_href)
                // A bool renders as an HTML boolean attribute, which is present or
                // absent. ARIA reads the literal strings instead.
                aria-pressed=(if pinned { "true" } else { "false" })
                title=(if pinned { "Unpin" } else { "Pin to the top" })
                class=(class!(
                    "mt-0.5 shrink-0 rounded-md border px-1.5 py-1 text-xs leading-none transition-colors",
                    "border-amber-400/50 bg-amber-400/10 text-amber-300" if pinned
                        else "border-slate-700 text-slate-500 hover:border-slate-600 hover:text-slate-300",
                ))
            >
                "pin"
            </a>

            <div class="min-w-0 flex-1">
                filename(segments: &matched.segments, ruleset: ruleset)

                <div class="mt-1.5 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-slate-500">
                    <span class=(class!(
                        "rounded-full px-2 py-0.5",
                        matched.diff.badge_classes(),
                    ))>
                        (matched.diff.label())
                    </span>
                    <span>(&matched.feed)</span>
                </div>
            </div>
        </li>
    }
}

/// What the page worked out about one listed release.
///
/// The values arrive rendered rather than raw. The clock, the feed registry,
/// and the rulesets all live outside this module, so the page resolves them
/// and this carries the answers.
pub(crate) struct ItemDetails {
    /// Every ruleset that claims the title, most specific first.
    ///
    /// A base and the child that narrows it both claim the same release.
    /// Listing only the winner makes the base read as a ruleset that never
    /// fires.
    pub rulesets: Vec<Claimant>,

    /// What the claiming ruleset read out of the title, in its field order.
    ///
    /// Empty when no ruleset claims the row. This is what a reader checks
    /// when a rule misfires: which run of the name became which part.
    pub values: Vec<ParsedValue>,

    /// Names why the row is not wanted, or nothing when it is.
    ///
    /// The reason is what a reader who asked to see everything came for: a
    /// row they cannot act on still owes them an explanation of why it sits
    /// there.
    pub hidden: Option<&'static str>,

    /// What the feed the row came from is called.
    pub feed_name: String,

    pub size: String,
    pub age: String,

    /// How the last grab of this release went, or nothing when none was
    /// tried.
    pub grab: Option<Grabbed>,
}

/// What became of the last grab of one release.
///
/// The age arrives rendered, because the row has no clock. That matches
/// [`ItemDetails::age`], which is rendered for the same reason.
pub(crate) struct Grabbed {
    /// Why the grab failed, or nothing when the client accepted it.
    pub error: Option<String>,

    /// How long ago the attempt was made.
    pub age: String,

    /// The ids of the rulesets that claimed the release when it was
    /// grabbed, most specific first.
    ///
    /// Kept apart from [`ItemDetails::rulesets`], which says what claims the
    /// title now. The two agree while the rulesets are static, and they part
    /// the moment a rule changes.
    pub rulesets: Vec<String>,
}

/// One stored feed item, prefixed by the control that selects it.
///
/// A row the page did not want dims rather than disappears, because a reader
/// who asked to see everything still has to tell it from what they want.
///
/// The title renders as plain monospace rather than tinted by part. Tinting
/// needs the segments the ruleset editor works from, which are checked-in
/// data rather than anything the engine produces from a real title.
#[component]
pub(crate) async fn item_row(
    engine: &Engine,
    item: &StoredItem,
    details: &ItemDetails,
    #[into] toggle_href: String,
    selected: bool,
) -> Result {
    view! {
        <li
            id=(format!("item-{}", item.id))
            class=(class!(
                "flex items-start gap-3 rounded-lg border px-4 py-3 scroll-mt-24 transition-colors",
                "border-sky-400/50 bg-sky-400/5" if selected
                    else "border-slate-800 bg-slate-900/40 hover:border-slate-700",
                "opacity-60" if details.hidden.is_some(),
            ))
        >
            checkbox(href: toggle_href, checked: selected, label: "Select this release")

            <div class="min-w-0 flex-1">
                <div class="flex flex-wrap items-center gap-2">
                    <span class="font-mono text-sm break-all">(&item.item.title)</span>
                    if let Some(label) = details.hidden {
                        <span class="rounded-full bg-amber-500/15 px-2 py-0.5 text-xs text-amber-300">
                            (label)
                        </span>
                    }
                    match &details.grab {
                        Some(grabbed) => match &grabbed.error {
                            None => <span
                                title=(&grabbed.age)
                                class="rounded-full bg-sky-400/15 px-2 py-0.5 text-xs text-sky-300"
                            >"grabbed"</span>,
                            Some(error) => <span
                                title=(error)
                                class="rounded-full bg-rose-500/15 px-2 py-0.5 text-xs text-rose-300"
                            >"grab failed"</span>,
                        },
                        None => "",
                    }
                </div>

                if !details.values.is_empty() {
                    <div class="mt-1.5 flex flex-wrap gap-1">
                        for value in &details.values {
                            <span
                                title=(if value.identity {
                                    format!("{} (identity)", value.name)
                                } else {
                                    value.name.to_owned()
                                })
                                class=(class!(
                                    "rounded-sm px-1.5 py-0.5 font-mono text-xs",
                                    value.part.classes(),
                                    "ring-1 ring-current/40" if value.identity else "opacity-70",
                                ))
                            >
                                (&value.value)
                            </span>
                        }
                    </div>
                }

                <div class="mt-2 flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-slate-500">
                    if details.rulesets.is_empty() {
                        <span class="text-slate-600">"unmatched"</span>
                    } else {
                        for ruleset in &details.rulesets {
                            <a
                                href=(format!("/admin/rulesets/{}", ruleset.id))
                                class="underline decoration-slate-700 underline-offset-2 hover:text-slate-300"
                            >
                                (&ruleset.name)
                            </a>
                        }
                    }
                    <span>(&details.feed_name)</span>
                    <span>(&details.size)</span>
                    <span>
                        if let Some(seeders) = item.item.seeders {
                            (format::count(seeders as usize, "seeder", "seeders"))
                        } else {
                            "seeders unknown"
                        }
                    </span>
                    <span>(&details.age)</span>
                </div>

                if let Some(grabbed) = &details.grab {
                    <p class="mt-1 text-xs text-slate-500">
                        if grabbed.rulesets.is_empty() {
                            "passed no ruleset"
                        } else {
                            "passed " (passed(engine, &grabbed.rulesets))
                        }
                    </p>
                }
            </div>
        </li>
    }
}

/// Names the rulesets a grab passed, in the order they were recorded.
///
/// A ruleset removed since the grab shows by its id instead of its name. The
/// record is of what ran, and hiding a line because a rule no longer exists
/// loses exactly the case a reader opened the page to look into.
fn passed(engine: &Engine, rulesets: &[String]) -> String {
    rulesets
        .iter()
        .map(|id| {
            engine
                .ruleset(id)
                .map_or(id.as_str(), |ruleset| ruleset.name.as_str())
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// A checkbox that toggles by navigation rather than by script.
///
/// The control is a link because the selection lives in the query string. A
/// real `input` would hold the state in the browser instead, where a reload
/// drops it and no one can share the result.
#[component]
pub(crate) async fn checkbox(#[into] href: String, checked: bool, label: &str) -> Result {
    view! {
        <a
            href=(href)
            role="checkbox"
            aria-checked=(if checked { "true" } else { "false" })
            aria-label=(label)
            class=(class!(
                "mt-0.5 flex size-4 shrink-0 items-center justify-center rounded border text-[10px] leading-none transition-colors",
                "border-sky-400 bg-sky-400 text-slate-900" if checked
                    else "border-slate-600 text-transparent hover:border-slate-400",
            ))
        >
            "x"
        </a>
    }
}
/// One extraction rule inside the ruleset editor.
///
/// The row is anchored by its part rather than its field name, because that
/// is what a highlighted run in a filename knows about itself. This holds
/// only while each part has at most one field. A second field on the same
/// part makes the anchor ambiguous, and the anchor must move to the name.
///
/// An inherited row dims and locks its inputs. The value shown is the
/// parent's, and editing it here would suggest a change this ruleset does not
/// hold. The trailing column is the way in: it replaces the parent's value
/// with one this ruleset owns.
#[component]
pub(crate) async fn field_row(
    index: usize,
    resolved: ResolvedField<'_>,
    #[into] toggle_href: String,
    inheriting: bool,
) -> Result {
    let field = resolved.field;
    let locked = resolved.is_inherited();

    view! {
        <div
            id=(format!("field-{}", field.part.slug()))
            class=(class!(
                "grid scroll-mt-24 grid-cols-1 gap-3 border-t border-slate-800 px-4 py-3 target:bg-slate-800/40 md:grid-cols-12 md:items-center",
                "opacity-45" if locked,
            ))
        >
            <div class="md:col-span-2">
                <label class="block text-xs text-slate-500">"Name"</label>
                <div class="mt-1 flex items-center gap-2">
                    <span class=(class!("size-2 shrink-0 rounded-full", field.part.dot()))></span>
                    <input
                        type="text"
                        name=(format!("field.{index}.name"))
                        value=(&field.name)
                        disabled=(locked)
                        class="w-full rounded-md border border-slate-800 bg-slate-950 px-2 py-1.5 font-mono text-sm text-slate-100 focus:border-slate-600 focus:outline-none"
                    >
                </div>
            </div>

            <div class="md:col-span-2">
                <label class="block text-xs text-slate-500">"Part"</label>
                <select
                    name=(format!("field.{index}.part"))
                    disabled=(locked)
                    class="mt-1 w-full rounded-md border border-slate-800 bg-slate-950 px-2 py-1.5 text-sm text-slate-100 focus:border-slate-600 focus:outline-none"
                >
                    for part in Part::ALL {
                        <option value=(part.slug()) selected=(*part == field.part)>
                            (part.slug())
                        </option>
                    }
                </select>
            </div>

            <div class="md:col-span-2">
                <label class="block text-xs text-slate-500">"Type"</label>
                <select
                    name=(format!("field.{index}.kind"))
                    disabled=(locked)
                    class="mt-1 w-full rounded-md border border-slate-800 bg-slate-950 px-2 py-1.5 text-sm text-slate-100 focus:border-slate-600 focus:outline-none"
                >
                    for kind in FieldKind::ALL {
                        <option value=(kind.label()) selected=(*kind == field.kind)>
                            (kind.label())
                        </option>
                    }
                </select>
            </div>

            <div class=(class!("md:col-span-2" if inheriting else "md:col-span-4"))>
                <label class="block text-xs text-slate-500">"Pattern"</label>
                <input
                    type="text"
                    name=(format!("field.{index}.pattern"))
                    value=(field.matcher())
                    disabled=(locked || field.pattern.is_none())
                    title=(field.pattern.is_none().then_some("The kind supplies this pattern"))
                    class="mt-1 w-full rounded-md border border-slate-800 bg-slate-950 px-2 py-1.5 font-mono text-xs text-slate-300 focus:border-slate-600 focus:outline-none"
                >
                match resolved.source {
                    FieldSource::Overridden { parent } => <p
                        class="mt-1 truncate font-mono text-[11px] text-slate-600"
                        title=(parent.matcher())
                    >
                        "replaces " (parent.matcher())
                    </p>,
                    _ => "",
                }
            </div>

            <div class="md:col-span-1 md:justify-self-center">
                <label class="block text-xs text-slate-500">"Required"</label>
                <input
                    type="checkbox"
                    name=(format!("field.{index}.required"))
                    checked=(field.required)
                    disabled=(locked)
                    class="mt-2 size-4 rounded border-slate-700 bg-slate-950"
                >
            </div>

            <div class="md:col-span-1 md:justify-self-center">
                <label
                    class="block text-xs text-slate-500"
                    title="Part of what decides whether two releases are the same item"
                >
                    "Identity"
                </label>
                <input
                    type="checkbox"
                    name=(format!("field.{index}.identity"))
                    checked=(field.identity)
                    disabled=(locked)
                    class="mt-2 size-4 rounded border-slate-700 bg-slate-950"
                >
            </div>

            if inheriting {
                <div class="md:col-span-2 md:justify-self-end">
                    <label class="block text-xs text-slate-500">"Source"</label>
                    <a
                        href=(toggle_href)
                        class=(class!(
                            "mt-1 inline-block rounded-md border px-2 py-1 text-xs transition-colors",
                            "border-slate-700 text-slate-400 hover:border-slate-500 hover:text-slate-200" if locked
                                else "border-sky-400/50 bg-sky-400/10 text-sky-300 hover:bg-sky-400/20",
                        ))
                    >
                        if locked { "inherited" } else { "replaced" }
                    </a>
                </div>
            }
        </div>
    }
}

/// The button that enables or disables a ruleset.
///
/// The label names the action rather than the state, which is what a button
/// beside Save and Add field reads as. [`status_badge`] carries the state.
///
/// The control posts rather than links, because it changes stored state. A
/// link would let a prefetch or a crawler disable a ruleset.
#[component]
pub(crate) async fn status_toggle(enabled: bool, #[into] action: String) -> Result {
    view! {
        <form method="post" action=(action) class="contents">
            <button
                type="submit"
                title=(if enabled {
                    "Stop this ruleset filtering feed results"
                } else {
                    "Let this ruleset filter feed results"
                })
                class=(class!(
                    "cursor-pointer rounded-md border px-3 py-1.5 text-sm transition-colors",
                    "border-emerald-500/40 bg-emerald-500/10 text-emerald-300 hover:bg-emerald-500/20" if enabled
                        else "border-slate-700 bg-slate-800/40 text-slate-400 hover:border-slate-600 hover:text-slate-200",
                ))
            >
                if enabled { "Disable" } else { "Enable" }
            </button>
        </form>
    }
}
/// A button that posts to `action`, for a write with nothing to configure.
///
/// The form wraps a single button because the write needs no input beyond the
/// action itself. A link lets a crawler or a prefetch trigger the write.
#[component]
pub(crate) async fn action_button(#[into] action: String, label: &str) -> Result {
    view! {
        <form method="post" action=(action) class="contents">
            <button
                type="submit"
                class="cursor-pointer rounded-md border border-slate-700 bg-slate-800/40 px-3 py-1.5 text-sm text-slate-400 transition-colors hover:border-slate-600 hover:text-slate-200"
            >
                (label)
            </button>
        </form>
    }
}

/// A link styled like [`action_button`], for a read that runs on navigation.
///
/// A form is right for a write and wrong for a read. A POST prompts the
/// reader to resubmit on reload, and a link does not.
#[component]
pub(crate) async fn link_button(#[into] href: String, label: &str) -> Result {
    view! {
        <a
            href=(href)
            class="cursor-pointer rounded-md border border-slate-700 bg-slate-800/40 px-3 py-1.5 text-sm text-slate-400 transition-colors hover:border-slate-600 hover:text-slate-200"
        >
            (label)
        </a>
    }
}

/// One ruleset on the admin index.
///
/// A `nested` card is a child, indented under the base it narrows.
///
/// The whole card is one link, so the badge here reports the state rather than
/// changing it. An anchor inside an anchor is not something a browser resolves
/// the way a reader expects, and the switch belongs beside the ruleset's other
/// actions in the editor.
#[component]
pub(crate) async fn ruleset_card(
    ruleset: &Ruleset,
    parent: Option<&Ruleset>,
    nested: bool,
    enabled: bool,
) -> Result {
    view! {
        <li
            id=(format!("ruleset-{}", ruleset.id))
            class=(class!("scroll-mt-24", "md:pl-8" if nested))
        >
            <a
                href=(format!("/admin/rulesets/{}", ruleset.id))
                class=(class!(
                    "block rounded-lg border bg-slate-900/40 px-4 py-4 transition-colors hover:border-slate-700",
                    "border-slate-800/70 border-l-2 border-l-slate-700" if nested
                        else "border-slate-800",
                ))
            >
                <div class="flex flex-wrap items-center gap-3">
                    <h2 class="text-sm font-semibold text-slate-100">(&ruleset.name)</h2>
                    status_badge(enabled: enabled)
                    match parent {
                        Some(parent) => <span class="rounded-full bg-slate-800/70 px-2 py-0.5 text-xs text-slate-400">
                            "narrows " (&parent.name)
                        </span>,
                        None => "",
                    }
                </div>

                <div class="mt-3 flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-slate-500">
                    match parent {
                        Some(parent) => <span>
                            (ruleset.fields.len()) " replaced of " (format::count(parent.fields.len(), "field", "fields"))
                        </span>,
                        None => <span>(format::count(ruleset.fields.len(), "field", "fields"))</span>,
                    }
                </div>
            </a>
        </li>
    }
}

/// Reports how a feed's last check went.
///
/// A feed with no check yet reads as neither good nor bad, because nothing
/// has been tried. That is a different state from a check that failed, and
/// the colors keep them apart at a glance.
#[component]
pub(crate) async fn check_badge(check: Option<&FeedCheck>) -> Result {
    view! {
        <span class=(class!(
            "rounded-full px-2 py-0.5 text-xs",
            match check.map(|check| check.outcome.is_ok()) {
                Some(true) => "bg-emerald-500/15 text-emerald-300",
                Some(false) => "bg-rose-500/15 text-rose-300",
                None => "bg-slate-700/40 text-slate-400",
            },
        ))>
            match check.map(|check| check.outcome.is_ok()) {
                Some(true) => "ok",
                Some(false) => "failed",
                None => "unchecked",
            }
        </span>
    }
}

/// Reports whether a ruleset runs, without changing it.
///
/// Used where the badge sits inside a larger link. The switch that changes the
/// state is [`status_toggle`].
#[component]
pub(crate) async fn status_badge(enabled: bool) -> Result {
    view! {
        <span class=(class!(
            "rounded-full px-2 py-0.5 text-xs",
            "bg-emerald-500/15 text-emerald-300" if enabled
                else "bg-slate-700/40 text-slate-400",
        ))>
            if enabled { "enabled" } else { "disabled" }
        </span>
    }
}

#[cfg(test)]
mod tests {
    use super::passed;
    use crate::ruleset::fixture::ENGINE;

    #[test]
    fn passed_names_declared_rulesets_and_keeps_the_rest_by_id() {
        assert_eq!(
            passed(
                &ENGINE,
                &[
                    "series-hollow-meridian".to_owned(),
                    "removed-since-the-grab".to_owned(),
                ]
            ),
            "The Hollow Meridian, removed-since-the-grab",
            "a ruleset no longer declared still shows, by the id that was recorded"
        );
    }
}
