//! Leptos UI components for the i18n demo.
//!
//! Demonstrates compile-time-checked translations with `leptos_i18n`:
//! - `t!(i18n, greeting, name = …)` — interpolation with a named argument.
//! - `t!(i18n, click_count, count = …)` — interpolation with a numeric count.
//! - `t_string!(i18n, …)` — translation returned as a `String` for attributes.
//! - `i18n.set_locale(…)` — runtime locale switching (EN / DE).

use crate::i18n::{Locale, t, t_string, use_i18n};
use leptos::prelude::*;
#[allow(
    clippy::wildcard_imports,
    reason = "leptos_meta re-exports are feature-gated (ssr/csr/hydrate); wildcard avoids conditional import errors when features change"
)]
use leptos_meta::*;
use leptos_router::components::{FlatRoutes, Route, Router};
use leptos_router::path;

// ---------------------------------------------------------------------------
// App shell – used by the server for SSR
// ---------------------------------------------------------------------------

/// HTML shell rendered by the server during SSR.
#[must_use]
pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8" />
                <meta name="viewport" content="width=device-width, initial-scale=1" />
                <AutoReload options=options.clone() />
                <HydrationScripts options />
                <MetaTags />
            </head>
            <body>
                <App />
            </body>
        </html>
    }
}

// ---------------------------------------------------------------------------
// Root component
// ---------------------------------------------------------------------------

/// Root component that provides meta context and i18n context, then renders
/// the router with i18n-aware pages.
#[expect(
    clippy::must_use_candidate,
    reason = "Leptos component returns impl IntoView; must_use is implicit"
)]
#[allow(non_snake_case)]
pub fn App() -> impl IntoView {
    provide_meta_context();
    let i18n = use_i18n();
    let title = move || t_string!(i18n, app_title);

    view! {
        <Stylesheet href="/pkg/i18n-demo.css" />
        // The Title component accepts a signal-like value for text.
        <Title text=title />

        <Router>
            <FlatRoutes fallback=|| view! { <p>"Page not found."</p> }>
                <Route path=path!("/") view=Home />
            </FlatRoutes>
        </Router>
    }
}

// ---------------------------------------------------------------------------
// Home page – i18n demo
// ---------------------------------------------------------------------------

/// Home page demonstrating `t!` macro usage and locale switching.
#[expect(
    clippy::must_use_candidate,
    reason = "Leptos component returns impl IntoView; must_use is implicit"
)]
#[allow(non_snake_case)]
pub fn Home() -> impl IntoView {
    let i18n = use_i18n();

    let (counter, set_counter) = signal(0u32);
    let inc = move |_| set_counter.update(|c| *c += 1);
    let count = move || counter.get();

    let on_switch = move |_| {
        let new_locale = match i18n.get_locale() {
            Locale::en => Locale::de,
            Locale::de => Locale::en,
        };
        i18n.set_locale(new_locale);
    };

    let placeholder = move || t_string!(i18n, search_placeholder);

    view! {
        <h1>{t!(i18n, greeting, name = "World")}</h1>

        <p>
            <button on:click=on_switch>
                {t!(i18n, search_button)}
            </button>
        </p>

        <p>
            <input type="text" placeholder={placeholder} />
        </p>

        <p>
            <button on:click=inc>
                {"+1"}
            </button>
            {" "}
            {t!{ i18n, click_count, count = count() }}
        </p>
    }
}
