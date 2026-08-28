use topcoat::{Result, dev, router::layout, tailwind, view::view};

/// Wraps every page in the HTML document shell.
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
            </head>
            <body class="min-h-screen bg-slate-950 text-slate-100 antialiased">
                (slot?)
            </body>
        </html>
    }
}
