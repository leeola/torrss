use topcoat::{Result, router::page, view::view};

#[page("/")]
async fn home() -> Result {
    view! {
        <main class="mx-auto flex min-h-screen max-w-2xl flex-col justify-center gap-4 px-6">
            <h1 class="text-4xl font-semibold tracking-tight">"Hello, world!"</h1>
            <p class="text-lg text-slate-400">"Torrss is running."</p>
        </main>
    }
}
