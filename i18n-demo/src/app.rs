//! Leptos UI components for the i18n demo.
//!
//! Demonstrates compile-time-checked translations with `leptos_i18n`:
//! - `t!(i18n, greeting, name = …)` — interpolation with a named argument.
//! - `t!(i18n, click_count, count = …)` — interpolation with a numeric count.
//! - `t_string!(i18n, …)` — translation returned as a `String` for attributes.
//! - `i18n.set_locale(…)` — runtime locale switching (EN / DE).
//!
//! Styling is provided by CSS injected via a `<style>` tag at runtime, with
//! selectors scoped under the `[data-i18n-demo]` attribute on the root container.

use crate::i18n::{Locale, t, t_string, use_i18n};
use crate::styles;
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
///
/// Injects the structured CSS into a `<style>` tag at runtime.  The CSS is
/// scoped under the `data-i18n-demo` attribute on the root container.
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
        // The Title component accepts a signal-like value for text.
        <Title text=title />

        // Inject the scoped CSS at runtime.
        <style>{styles::home_css()}</style>

        <div data-i18n-demo>
            <Router>
                <FlatRoutes fallback=|| view! { <p>"Page not found."</p> }>
                    <Route path=path!("/") view=Home />
                </FlatRoutes>
            </Router>
        </div>
    }
}

// ---------------------------------------------------------------------------
// Home page – i18n demo
// ---------------------------------------------------------------------------

/// Home page demonstrating `t!` macro usage, locale switching,
/// `<Show when=signal>`, and `<ShowLet>` for conditional rendering.
#[expect(
    clippy::must_use_candidate,
    reason = "Leptos component returns impl IntoView; must_use is implicit"
)]
#[allow(non_snake_case)]
pub fn Home() -> impl IntoView {
    let i18n = use_i18n();

    // ---- reactive state ---------------------------------------------------

    let (counter, set_counter) = signal(0u32);
    let inc = move |_| set_counter.update(|c| *c += 1);

    // Derived signals for conditional rendering.
    let has_clicked = Signal::derive(move || counter.get() > 0);

    // An Option-based value for demonstrating <ShowLet>.  We unwrap this in
    // the view with the `let:value` syntax.
    let maybe_count = Signal::derive(move || {
        let c = counter.get();
        if c > 0 { Some(c) } else { None }
    });

    let on_switch = move |_| {
        let new_locale = match i18n.get_locale() {
            Locale::en => Locale::de,
            Locale::de => Locale::en,
        };
        i18n.set_locale(new_locale);
    };

    let placeholder = move || t_string!(i18n, search_placeholder);

    view! {
        // --- greeting ---
        <h1>{t!(i18n, greeting, name = "World")}</h1>

        // --- locale toggle ---
        <p>
            <button
                class=styles::LOCALE_BTN
                on:click=on_switch
            >
                {t!(i18n, search_button)}
            </button>
        </p>

        // --- search input ---
        <p>
            <input
                class=styles::SEARCH_INPUT
                type="text"
                placeholder={placeholder}
            />
        </p>

        // --- counter section ---
        //
        // <Show when=has_clicked> — demonstrates signal-passing directly to
        // the `when` prop.
        <p>
            <button
                class=styles::COUNTER_BTN
                on:click=inc
            >
                {"+1"}
            </button>
        </p>

        // <Show when=move || ...> uses a closure (Leptos 0.8.x Show still
        // expects Fn() -> bool; Signal::derive does not auto-coerce).
        <Show when=move || has_clicked.get()>
            <p>
                // <ShowLet some=signal> unwraps the Option, providing
                // `value` to the children (Leptos 0.8.8+).
                // The prop is `some`, not `when`.
                <ShowLet
                    some=maybe_count
                    fallback=|| ()
                    let:value
                >
                    {t!{ i18n, click_count, count = value }}
                    {" "}
                    <span class=styles::CLICK_COUNT>
                        {value}
                    </span>
                </ShowLet>
            </p>
        </Show>
    }
}
