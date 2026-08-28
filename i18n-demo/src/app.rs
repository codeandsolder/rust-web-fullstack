//! Leptos UI components for the i18n demo.
//!
//! Demonstrates compile-time-checked translations with `leptos_i18n`, runtime
//! EN/DE locale switching, and reactive conditional rendering.

use crate::i18n::{Locale, t, t_string, use_i18n};
use crate::styles;
use leptos::hydration::{AutoReload, HydrationScripts};
use leptos::prelude::*;
use leptos_meta::{MetaTags, Title, provide_meta_context};
use leptos_router::components::{FlatRoutes, Route, Router};
use leptos_router::path;

/// HTML shell rendered by the server during SSR. English is the initial locale;
/// the hydrated client updates the root `lang` attribute when locale changes.
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

#[cfg(target_arch = "wasm32")]
fn set_document_lang(lang: &str) {
    let Some(document) = leptos::web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let Some(root) = document.document_element() else {
        return;
    };
    if root.set_attribute("lang", lang).is_err() {
        leptos::logging::warn!("failed to update document lang attribute");
    }
}

#[cfg(not(target_arch = "wasm32"))]
const fn set_document_lang(_lang: &str) {}

#[expect(
    clippy::must_use_candidate,
    reason = "Leptos component returns impl IntoView; must_use is implicit"
)]
#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();
    let i18n = use_i18n();
    let title = move || t_string!(i18n, app_title);

    view! {
        <Title text=title />
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

#[expect(
    clippy::must_use_candidate,
    reason = "Leptos component returns impl IntoView; must_use is implicit"
)]
#[component]
pub fn Home() -> impl IntoView {
    let i18n = use_i18n();

    let (counter, set_counter) = signal(0u32);
    let inc = move |_| set_counter.update(|c| *c += 1);
    let has_clicked = Signal::derive(move || counter.get() > 0);
    let maybe_count = Signal::derive(move || {
        let count = counter.get();
        (count > 0).then_some(count)
    });

    let on_switch = move |_| {
        let (new_locale, lang) = match i18n.get_locale() {
            Locale::en => (Locale::de, "de"),
            Locale::de => (Locale::en, "en"),
        };
        set_document_lang(lang);
        i18n.set_locale(new_locale);
    };

    let placeholder = move || t_string!(i18n, search_placeholder);

    view! {
        <h1>{t!(i18n, greeting, name = "World")}</h1>

        <p>
            <button class=styles::LOCALE_BTN on:click=on_switch>
                {t!(i18n, switch_language)}
            </button>
        </p>

        <p>
            <input class=styles::SEARCH_INPUT type="text" placeholder=placeholder />
        </p>

        <p>
            <button class=styles::COUNTER_BTN on:click=inc>
                "+1"
            </button>
        </p>

        <Show when=move || has_clicked.get()>
            <p>
                <ShowLet some=maybe_count fallback=|| () let:value>
                    {t!(i18n, click_count, count = value)}
                    " "
                    <span class=styles::CLICK_COUNT>{value}</span>
                </ShowLet>
            </p>
        </Show>
    }
}
