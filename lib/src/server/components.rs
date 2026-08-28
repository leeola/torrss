use topcoat::{
    Result,
    view::{class, component, view},
};

use crate::mock::{Candidate, FieldKind, FieldSource, Release, ResolvedField, Ruleset, Segment};

/// Renders a filename with every claimed run tinted by its part.
///
/// Each claimed run links to the field that produced it, anchored inside
/// `ruleset`'s editor, so a reader jumps from a value to the rule behind it.
#[component]
pub(crate) async fn filename(segments: &'static [Segment], ruleset: &str) -> Result {
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

/// One candidate filename in the editor, tinted by how the edit changed it.
///
/// The pin control is a link rather than a button because pinning lives in
/// the query string. That keeps the pinned set shareable and survives the
/// reload that follows every pattern edit.
#[component]
pub(crate) async fn candidate_row(
    candidate: &'static Candidate,
    ruleset: &str,
    #[into] pin_href: String,
    pinned: bool,
) -> Result {
    view! {
        <li
            id=(format!("match-{}", candidate.id))
            class=(class!(
                "flex items-start gap-3 rounded-lg border px-3 py-2.5 scroll-mt-24",
                candidate.diff.row_classes(),
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
                filename(segments: candidate.segments, ruleset: ruleset)

                <div class="mt-1.5 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-slate-500">
                    <span class=(class!(
                        "rounded-full px-2 py-0.5",
                        candidate.diff.badge_classes(),
                    ))>
                        (candidate.diff.label())
                    </span>
                    <span>(candidate.feed)</span>
                </div>
            </div>
        </li>
    }
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

/// One feed result, prefixed by the control that selects it.
#[component]
pub(crate) async fn release_row(
    release: &'static Release,
    #[into] toggle_href: String,
    selected: bool,
) -> Result {
    view! {
        <li
            id=(format!("release-{}", release.id))
            class=(class!(
                "flex items-start gap-3 rounded-lg border px-4 py-3 scroll-mt-24 transition-colors",
                "border-sky-400/50 bg-sky-400/5" if selected
                    else "border-slate-800 bg-slate-900/40 hover:border-slate-700",
            ))
        >
            checkbox(href: toggle_href, checked: selected, label: "Select this release")

            <div class="min-w-0 flex-1">
                filename(segments: release.segments, ruleset: release.ruleset)

                <div class="mt-2 flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-slate-500">
                    <a
                        href=(format!("/admin/rulesets/{}", release.ruleset))
                        class="text-slate-400 underline decoration-slate-700 underline-offset-2 hover:text-slate-200"
                    >
                        (release.ruleset)
                    </a>
                    <span>(release.feed)</span>
                    <span>(release.size)</span>
                    <span>(release.seeders) " seeders"</span>
                    <span>(release.age)</span>
                </div>
            </div>
        </li>
    }
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
    resolved: ResolvedField,
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
            <div class="md:col-span-3">
                <label class="block text-xs text-slate-500">"Name"</label>
                <div class="mt-1 flex items-center gap-2">
                    <span class=(class!("size-2 shrink-0 rounded-full", field.part.dot()))></span>
                    <input
                        type="text"
                        name=(format!("{}.name", field.name))
                        value=(field.name)
                        disabled=(locked)
                        class="w-full rounded-md border border-slate-800 bg-slate-950 px-2 py-1.5 font-mono text-sm text-slate-100 focus:border-slate-600 focus:outline-none"
                    >
                </div>
            </div>

            <div class="md:col-span-2">
                <label class="block text-xs text-slate-500">"Type"</label>
                <select
                    name=(format!("{}.kind", field.name))
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

            <div class=(class!("md:col-span-4" if inheriting else "md:col-span-6"))>
                <label class="block text-xs text-slate-500">"Pattern"</label>
                <input
                    type="text"
                    name=(format!("{}.pattern", field.name))
                    value=(field.pattern)
                    disabled=(locked)
                    class="mt-1 w-full rounded-md border border-slate-800 bg-slate-950 px-2 py-1.5 font-mono text-xs text-slate-300 focus:border-slate-600 focus:outline-none"
                >
                match resolved.source {
                    FieldSource::Overridden { parent } => <p
                        class="mt-1 truncate font-mono text-[11px] text-slate-600"
                        title=(parent.pattern)
                    >
                        "replaces " (parent.pattern)
                    </p>,
                    _ => "",
                }
            </div>

            <div class="md:col-span-1 md:justify-self-center">
                <label class="block text-xs text-slate-500">"Required"</label>
                <input
                    type="checkbox"
                    name=(format!("{}.required", field.name))
                    checked=(field.required)
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
/// One ruleset on the admin index.
///
/// A `nested` card is a child, indented under the base it narrows.
///
/// The whole card is one link, so the badge here reports the state rather than
/// changing it. An anchor inside an anchor is not something a browser resolves
/// the way a reader expects, and the switch belongs beside the ruleset's other
/// actions in the editor.
#[component]
pub(crate) async fn ruleset_card(ruleset: &'static Ruleset, nested: bool, enabled: bool) -> Result {
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
                    <h2 class="text-sm font-semibold text-slate-100">(ruleset.name)</h2>
                    status_badge(enabled: enabled)
                    match ruleset.parent() {
                        Some(parent) => <span class="rounded-full bg-slate-800/70 px-2 py-0.5 text-xs text-slate-400">
                            "narrows " (parent.name)
                        </span>,
                        None => "",
                    }
                </div>

                <p class="mt-1 text-sm text-slate-400">(ruleset.summary)</p>

                <div class="mt-3 flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-slate-500">
                    match ruleset.parent() {
                        Some(parent) => <span>
                            (ruleset.fields.len()) " replaced of " (parent.fields.len()) " fields"
                        </span>,
                        None => <span>(ruleset.fields.len()) " fields"</span>,
                    }
                    <span>(ruleset.match_count()) " matches"</span>
                    <span>(ruleset.feeds.join(", "))</span>
                </div>
            </a>
        </li>
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
