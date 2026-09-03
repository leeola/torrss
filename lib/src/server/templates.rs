use topcoat::{
    Result,
    context::Cx,
    dev,
    router::{layout, request::uri},
    runtime, tailwind,
    view::{class, component, view},
};

/// Wraps every page in the HTML document shell and the site header.
#[layout("/")]
async fn document(slot: Result) -> Result {
    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8">
                <meta name="viewport" content="width=device-width, initial-scale=1">
                <title>"Torrss"</title>
                <link rel="stylesheet" href=(tailwind::stylesheet!())>
                dev::script()
                runtime::script()
            </head>
            <body class="min-h-screen bg-slate-950 text-slate-100 antialiased">
                site_header()
                <main class="mx-auto w-full max-w-5xl px-6 py-10">
                    (slot?)
                </main>
            </body>
        </html>
    }
}

/// The bar every page shares, holding the wordmark and the primary nav.
#[component]
async fn site_header(cx: &Cx) -> Result {
    let path = uri(cx).path();

    view! {
        <header class="border-b border-slate-800 bg-slate-900/40">
            <div class="mx-auto flex w-full max-w-5xl items-center gap-8 px-6 py-4">
                <a href="/" class="text-base font-semibold tracking-tight text-slate-100">
                    "torrss"
                </a>
                <nav class="flex items-center gap-1 text-sm">
                    nav_link(href: "/", label: "Feed", current: path == "/")
                    nav_link(
                        href: "/admin",
                        label: "Rulesets",
                        current: path == "/admin" || path.starts_with("/admin/rulesets"),
                    )
                    nav_link(
                        href: "/admin/client",
                        label: "Client",
                        current: path == "/admin/client",
                    )
                    nav_link(
                        href: "/admin/feeds",
                        label: "Feeds",
                        current: path.starts_with("/admin/feeds"),
                    )
                </nav>
            </div>
        </header>
    }
}

#[component]
async fn nav_link(href: &str, label: &str, current: bool) -> Result {
    view! {
        <a
            href=(href)
            class=(class!(
                "rounded-md px-3 py-1.5 transition-colors",
                "bg-slate-800 text-slate-100" if current else "text-slate-400 hover:text-slate-200",
            ))
            if current {
                aria-current="page"
            }
        >
            (label)
        </a>
    }
}
