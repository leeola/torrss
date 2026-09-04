use topcoat::{
    Result,
    view::{class, component, view},
};

use super::format;
use super::listing::ParsedValue;
use super::matches::Match;
use crate::feed::store::FeedCheck;
use crate::rules::Engine;
use crate::ruleset::{
    Field, FieldKind, FieldSource, ResolvedField, Ruleset, RulesetTest, Segment, Tint,
};
use crate::store::StoredItem;
use crate::torrent::{Torrent, TorrentState};
use url::form_urlencoded;

/// Renders a filename with every claimed run tinted by the field that
/// claimed it.
///
/// Each claimed run links to that field's row, anchored inside `ruleset`'s
/// editor, so a reader jumps from a value to the rule behind it.
#[component]
pub(crate) async fn filename(segments: &[Segment<'_>], ruleset: &str) -> Result {
    view! {
        <span class="font-mono text-sm break-all">
            for segment in segments {
                match segment.field {
                    Some(position) => <a
                        href=(format!("/admin/rulesets/{ruleset}#field-{position}"))
                        class=(class!(
                            "rounded-sm px-0.5 py-px hover:underline hover:decoration-dotted",
                            Tint::at(position).classes(),
                        ))
                        title="edit the rule that claimed this"
                    >(segment.text)</a>,
                    None => <span class="text-slate-500">(segment.text)</span>,
                }
            }
        </span>
    }
}

/// One feed filter, naming the feed it narrows the listing to.
///
/// The chip carries its feed in its own value, so one delegated handler reads
/// every chip. An empty value is the whole set rather than a feed.
#[component]
pub(crate) async fn filter_chip(value: &str, label: &str, current: bool) -> Result {
    view! {
        <button
            type="button"
            name="feed-filter"
            value=(value)
            aria-pressed=(if current { "true" } else { "false" })
            class=(class!(
                "rounded-full border px-3 py-1 text-xs transition-colors",
                "border-slate-600 bg-slate-800 text-slate-100" if current
                    else "border-slate-800 text-slate-400 hover:border-slate-700 hover:text-slate-200",
            ))
        >
            (label)
        </button>
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
///
/// The chip names its state in its own value, so one delegated handler reads
/// every chip. An empty value is every state rather than one.
#[component]
pub(crate) async fn diff_filter(value: &str, label: &str, count: usize, current: bool) -> Result {
    view! {
        <button
            type="button"
            name="diff-filter"
            value=(value)
            aria-pressed=(if current { "true" } else { "false" })
            class=(class!(
                "rounded-full border px-3 py-1 text-xs transition-colors",
                "border-slate-600 bg-slate-800 text-slate-100" if current
                    else "border-slate-800 text-slate-400 hover:border-slate-700 hover:text-slate-200",
            ))
        >
            (label) " " <span class="text-slate-500">(count)</span>
        </button>
    }
}

/// One stored title in the editor, tinted by how the edit changed it.
///
/// The row carries what it read into the test it saves, so a reader states an
/// expectation by naming a title the rules already parse rather than by
/// typing the answer back in.
///
/// A row the draft does not claim saves too. It carries no expectations and
/// fails until the ruleset claims it, which is how a reader says "make this
/// match".
#[component]
pub(crate) async fn match_row(matched: &Match<'_>, ruleset: &str) -> Result {
    let payload = {
        let mut pairs = form_urlencoded::Serializer::new(String::new());

        pairs.append_pair("title", matched.title);
        for (field, value) in &matched.values {
            pairs.append_pair(&format!("expect.{field}"), value);
        }

        pairs.finish()
    };

    view! {
        <li
            id=(format!("match-{}", matched.id))
            class=(class!(
                "flex items-start gap-3 rounded-lg border px-3 py-2.5 scroll-mt-24",
                matched.diff.row_classes(),
            ))
        >
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

            <button
                type="button"
                name="row-action"
                value=(format!("test:{payload}"))
                class="shrink-0 rounded-md border border-sky-400/50 bg-sky-400/10 px-2 py-1 text-xs text-sky-300 transition-colors hover:bg-sky-400/20"
            >
                "save as test"
            </button>
        </li>
    }
}

/// What the page worked out about one listed release.
///
/// The values arrive rendered rather than raw. The clock, the feed registry,
/// and the rulesets all live outside this module, so the page resolves them
/// and this carries the answers.
pub(crate) struct ItemDetails {
    /// Every ruleset that claims the title, in declaration order.
    ///
    /// A template claims nothing, so it never appears. Two rulesets that both
    /// claim one release do, and the first declared is the one that parsed
    /// it, so listing all of them shows the reader the overlap they wrote.
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
    /// grabbed, in declaration order.
    ///
    /// Kept apart from [`ItemDetails::rulesets`], which says what claims the
    /// title now. The two agree while the rulesets are static, and they part
    /// the moment a rule changes.
    pub rulesets: Vec<String>,
}

/// One stored feed item, prefixed by the checkbox that selects it.
///
/// The checkbox posts nothing. The page reads it, keeps the selection in the
/// browser, and hands the set to the grab procedure, so a click changes one
/// box and nothing else.
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
    selected: bool,
) -> Result {
    view! {
        <li
            id=(format!("item-{}", item.id))
            class=(class!(
                "flex items-start gap-3 rounded-lg border border-slate-800 bg-slate-900/40 px-4 py-3 scroll-mt-24 transition-colors hover:border-slate-700 has-checked:border-sky-400/50 has-checked:bg-sky-400/5",
                "opacity-60" if details.hidden.is_some(),
            ))
        >
            <input
                type="checkbox"
                name="item"
                value=(item.id.to_string())
                checked=(selected)
                aria-label="Select this release"
                class="mt-1 size-4 shrink-0 rounded border-slate-700 bg-slate-950"
            >

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
                    parsed_chips(values: &details.values)
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

/// One extraction rule inside the ruleset editor.
///
/// The row is anchored by its part rather than its field name, because that
/// is what a highlighted run in a filename knows about itself. This holds
/// only while each part has at most one field. A second field on the same
/// part makes the anchor ambiguous, and the anchor must move to the name.
///
/// An inherited row dims and locks its inputs. The value shown is the
/// template's, and editing it here would suggest a change this ruleset does
/// not hold. The trailing column is the way in: it replaces the template's
/// value with one this ruleset owns.
///
/// `movable` gives the row its arrows. A row moves only where this ruleset
/// owns the order, which a ruleset built on a template does not.
#[component]
pub(crate) async fn field_row(
    index: usize,
    position: usize,
    movable: bool,
    resolved: ResolvedField<'_>,
) -> Result {
    let field = resolved.field;
    let locked = resolved.is_inherited();

    view! {
        <div
            id=(format!("field-{position}"))
            class=(class!(
                "grid scroll-mt-24 grid-cols-1 gap-3 border-t border-slate-800 px-4 py-3 target:bg-slate-800/40 md:grid-cols-12 md:items-center",
                "opacity-45" if locked,
            ))
            // The replace button copies these into a new own row, so an
            // inherited field becomes one this ruleset holds.
            data-name=(&field.name)
            data-kind=(field.kind.label())
            data-pattern=(field.matcher().unwrap_or_default())
            data-required=(if field.required { "on" } else { "" })
            data-identity=(if field.identity { "on" } else { "" })
        >
            <div class="md:col-span-2">
                <label class="block text-xs text-slate-500">"Name"</label>
                <div class="mt-1 flex items-center gap-2">
                    <span class=(class!("size-2 shrink-0 rounded-full", Tint::at(position).dot()))></span>
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

            <div class="md:col-span-4">
                <label class="block text-xs text-slate-500">"Pattern"</label>
                // The input locks only where the kind supplies the pattern and
                // the field takes it. A blank has nothing to take, and an
                // override is the reader's own text, so both stay editable.
                <input
                    type="text"
                    name=(format!("field.{index}.pattern"))
                    value=(field.matcher().unwrap_or_default())
                    disabled=(locked || (field.kind.pattern().is_some() && field.pattern.is_none()))
                    title=((field.kind.pattern().is_some() && field.pattern.is_none())
                        .then_some("The kind supplies this pattern"))
                    placeholder=(field.matcher().is_none().then_some(
                        "blank. The ruleset based on this template fills it in."
                    ))
                    class="mt-1 w-full rounded-md border border-slate-800 bg-slate-950 px-2 py-1.5 font-mono text-xs text-slate-300 focus:border-slate-600 focus:outline-none"
                >
                match resolved.source {
                    FieldSource::Overridden { template } => <p
                        class="mt-1 truncate font-mono text-[11px] text-slate-600"
                        title=(template.matcher().unwrap_or_default())
                    >
                        match template.matcher() {
                            Some(matcher) => { "replaces " (matcher) },
                            None => "fills in a blank",
                        }
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

            <div class="md:col-span-2 md:justify-self-end">
                <label class="block text-xs text-slate-500">"Source"</label>
                <div class="mt-1 flex items-center gap-1">
                    if movable {
                        <button
                            type="button"
                            name="row-action"
                            value=(format!("move-up:{index}"))
                            title="Move up"
                            class="rounded-md border border-slate-700 px-2 py-1 text-xs text-slate-400 transition-colors hover:border-slate-500 hover:text-slate-200"
                        >
                            "\u{2191}"
                        </button>
                        <button
                            type="button"
                            name="row-action"
                            value=(format!("move-down:{index}"))
                            title="Move down"
                            class="rounded-md border border-slate-700 px-2 py-1 text-xs text-slate-400 transition-colors hover:border-slate-500 hover:text-slate-200"
                        >
                            "\u{2193}"
                        </button>
                    }
                    <button
                        type="button"
                        // A `type="button"` never joins `FormData`, so the name
                        // stays out of the form encoding the shard parses.
                        name="row-action"
                        value=(if locked {
                            format!("replace:{}", field.name)
                        } else {
                            format!("remove:{index}")
                        })
                        class=(class!(
                            "inline-block rounded-md border px-2 py-1 text-xs transition-colors",
                            "border-slate-700 text-slate-400 hover:border-slate-500 hover:text-slate-200" if locked
                                else "border-sky-400/50 bg-sky-400/10 text-sky-300 hover:bg-sky-400/20",
                        ))
                    >
                        if locked { "replace" } else { "remove" }
                    </button>
                </div>
            </div>
        </div>
    }
}

/// One saved test inside the ruleset editor.
///
/// The row is anchored by its index rather than its title, because the title
/// is what the reader types and an anchor that moves with every keystroke
/// links to nothing.
///
/// One input per field, so the reader names the values they mean to assert
/// and leaves the rest alone. An empty input is not an assertion that the
/// field reads nothing.
#[component]
pub(crate) async fn test_row(index: usize, test: &RulesetTest, fields: &[&Field]) -> Result {
    view! {
        <div
            id=(format!("test-{index}"))
            class="grid scroll-mt-24 grid-cols-1 gap-3 border-t border-slate-800 px-4 py-3 target:bg-slate-800/40 md:grid-cols-12 md:items-center"
        >
            <div class="md:col-span-5">
                <label class="block text-xs text-slate-500">"Title"</label>
                <input
                    type="text"
                    name=(format!("test.{index}.title"))
                    value=(&test.title)
                    placeholder="The.Hollow.Meridian.S04E06.1080p"
                    class="mt-1 w-full rounded-md border border-slate-800 bg-slate-950 px-2 py-1.5 font-mono text-xs text-slate-100 focus:border-slate-600 focus:outline-none"
                >
            </div>

            for (position, field) in fields.iter().enumerate() {
                <div class="md:col-span-2">
                    <label class="flex items-center gap-2 text-xs text-slate-500">
                        <span class=(class!("size-2 shrink-0 rounded-full", Tint::at(position).dot()))></span>
                        (&field.name)
                    </label>
                    <input
                        type="text"
                        name=(format!("test.{index}.expect.{}", field.name))
                        value=(test.expected.get(&field.name).map_or("", String::as_str))
                        class="mt-1 w-full rounded-md border border-slate-800 bg-slate-950 px-2 py-1.5 font-mono text-xs text-slate-300 focus:border-slate-600 focus:outline-none"
                    >
                </div>
            }

            <div class="md:col-span-1 md:justify-self-end">
                <button
                    type="button"
                    // A `type="button"` never joins `FormData`, so the name
                    // stays out of the form encoding the shard parses.
                    name="row-action"
                    value=(format!("remove-test:{index}"))
                    class="mt-5 inline-block rounded-md border border-sky-400/50 bg-sky-400/10 px-2 py-1 text-xs text-sky-300 transition-colors hover:bg-sky-400/20"
                >
                    "remove"
                </button>
            </div>
        </div>
    }
}

/// A link that carries the reader to a page, styled as a button.
///
/// It reads rather than writes, so it is a link and not a control. A write
/// belongs to a procedure the page calls, which leaves the reader where they
/// are.
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
/// A `nested` card is indented under the template it is based on.
///
/// The whole card is one link, so the badge here reports the state rather than
/// changing it. An anchor inside an anchor is not something a browser resolves
/// the way a reader expects, and the switch belongs beside the ruleset's other
/// actions in the editor.
#[component]
pub(crate) async fn ruleset_card(
    ruleset: &Ruleset,
    template: Option<&Ruleset>,
    nested: bool,
    enabled: bool,
    is_template: bool,
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
                    // A template shows what it is rather than whether it runs,
                    // because it claims nothing and carries no switch.
                    if is_template {
                        <span class="rounded-full bg-slate-800/70 px-2 py-0.5 text-xs text-slate-400">
                            "template"
                        </span>
                    } else {
                        status_badge(enabled: enabled)
                    }
                    match template {
                        Some(template) => <span class="rounded-full bg-slate-800/70 px-2 py-0.5 text-xs text-slate-400">
                            "based on " (&template.name)
                        </span>,
                        None => "",
                    }
                </div>

                <div class="mt-3 flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-slate-500">
                    match template {
                        Some(template) => <span>
                            (ruleset.fields.len()) " replaced of " (format::count(template.fields.len(), "field", "fields"))
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
/// Used where the badge sits inside a larger link, which holds no control of
/// its own.
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

/// One torrent the client holds that a ruleset claims.
///
/// The ingest age arrives rendered, because the row has no clock. That matches
/// [`ItemDetails::age`], which is rendered for the same reason.
///
/// A torrent with no age was in the client before the store recorded any grab,
/// so the row says so rather than leaving a gap.
///
/// The row also shows what the ruleset read out of the name, as a release on
/// the home page does.
#[component]
pub(crate) async fn torrent_row(
    torrent: &Torrent,
    ruleset: &Claimant,
    values: &[ParsedValue],
    ingested: Option<&str>,
) -> Result {
    // Each tint is a whole literal, because the Tailwind scanner reads class
    // names out of source text and never sees one joined at runtime.
    let (word, tint) = match &torrent.state {
        TorrentState::Queued => ("queued", "bg-slate-500/15 text-slate-300"),
        TorrentState::Downloading => ("downloading", "bg-sky-400/15 text-sky-300"),
        TorrentState::Seeding => ("seeding", "bg-emerald-500/15 text-emerald-300"),
        TorrentState::Paused => ("paused", "bg-amber-500/15 text-amber-300"),
        TorrentState::Error(_) => ("error", "bg-rose-500/15 text-rose-300"),
    };

    let percent = format::percent(torrent.progress);

    view! {
        <li class="rounded-lg border border-slate-800 bg-slate-900/40 px-4 py-3">
            <div class="flex flex-wrap items-center gap-2">
                <span class="font-mono text-sm break-all">(&torrent.name)</span>
                <span class=(format!("rounded-full px-2 py-0.5 text-xs {tint}"))>(word)</span>
            </div>

            if !values.is_empty() {
                parsed_chips(values: values)
            }

            <div class="mt-2 flex items-center gap-2 text-xs text-slate-500">
                // The width is an inline style because it differs per row, and
                // only a literal class reaches the stylesheet.
                <div class="h-1 w-32 overflow-hidden rounded bg-slate-800">
                    <div class="h-1 bg-sky-400" style=(format!("width: {percent}"))></div>
                </div>
                <span>(&percent)</span>
            </div>

            <div class="mt-2 flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-slate-500">
                <a
                    href=(format!("/admin/rulesets/{}", ruleset.id))
                    class="underline decoration-slate-700 underline-offset-2 hover:text-slate-300"
                >
                    (&ruleset.name)
                </a>
                <span>(format::size(Some(torrent.size)))</span>
                if let TorrentState::Error(reason) = &torrent.state {
                    <span>(reason)</span>
                }
                <span>
                    match ingested {
                        Some(age) => { "ingested " (age) },
                        None => "held before any grab was recorded",
                    }
                </span>
            </div>
        </li>
    }
}

/// The values a ruleset read out of one name, one chip per field.
///
/// Each chip is tinted by its field's position and ringed when that field
/// decides whether two releases are the same. The field's name is on hover,
/// because a value reads for itself and a label beside every one crowds the
/// row.
///
/// The caller guards an empty list, so this renders one strip and a row with
/// nothing parsed adds no empty element.
#[component]
pub(crate) async fn parsed_chips(values: &[ParsedValue]) -> Result {
    view! {
        <div class="mt-1.5 flex flex-wrap gap-1">
            for value in values {
                <span
                    title=(if value.identity {
                        format!("{} (identity)", value.name)
                    } else {
                        value.name.to_owned()
                    })
                    class=(class!(
                        "rounded-sm px-1.5 py-0.5 font-mono text-xs",
                        Tint::at(value.position).classes(),
                        "ring-1 ring-current/40" if value.identity else "opacity-70",
                    ))
                >
                    (&value.value)
                </span>
            }
        </div>
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
