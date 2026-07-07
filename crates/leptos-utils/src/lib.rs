//! Shared [`axum`] / Leptos utilities used by SSR binaries.
//!
//! Currently provides [`probed_server_fn_handler`], the doubled-prefix probe
//! needed because Leptos 0.8's `#[server(endpoint = "/api/…")]` macro
//! registers the function at `/api/api/…` while the wire client calls
//! `/api/…`. Mount the handler at both `/api/{*fn_name}` and
//! `/api/api/{*fn_name}` and the probe will short-circuit to the registered
//! path on every request, eliminating the 404 the macro would otherwise cause.
//!
//! All items in this crate are SSR-only — the `leptos::server_fn::axum` and
//! `leptos_axum::handle_server_fns` paths they use are not compiled for the
//! `wasm32-unknown-unknown` target. We deliberately publish the crate as
//! `rlib` only (no `cdylib`) so it does not pull `leptos` into the WASM
//! transitive dependency graph.

use axum::body::Body;
use axum::extract::Request;
use axum::http::Uri;
use axum::response::IntoResponse;

/// Catch-all handler for Leptos server function endpoints.
///
/// Probes the exact path first via
/// [`leptos::server_fn::axum::get_server_fn_service`]; if the path is not
/// registered and it begins with `/api/`, the handler also probes the
/// doubled-prefix variant (`/api/api/…`) and rewrites the request URI in
/// place before delegating to [`leptos_axum::handle_server_fns`].
///
/// Mount it at both `/api/{*fn_name}` and `/api/api/{*fn_name}` so the
/// internal short-circuit handles every case without depending on the
/// rewrite path.
///
/// # Panics
///
/// Panics only if the path-rewrite produces an invalid URI — in practice this
/// is infallible because we only ever prepend `/api` to an existing valid URI.
#[expect(
    clippy::expect_used,
    reason = "Path rewrite produces a valid URI by construction (prepending /api to a valid path)"
)]
pub async fn probed_server_fn_handler(req: Request<Body>) -> impl IntoResponse {
    let method = req.method().clone();
    let original_path = req.uri().path().to_string();
    let (mut parts, body) = req.into_parts();

    let path_to_try =
        if leptos::server_fn::axum::get_server_fn_service(&original_path, method.clone()).is_none()
            && original_path.starts_with("/api/")
        {
            let doubled = format!("/api{original_path}");
            if leptos::server_fn::axum::get_server_fn_service(&doubled, method).is_some() {
                doubled
            } else {
                original_path
            }
        } else {
            original_path
        };

    if path_to_try != parts.uri.path() {
        parts.uri = Uri::try_from(&path_to_try).expect("valid URI from path rewrite");
    }

    let req = Request::from_parts(parts, body);
    leptos_axum::handle_server_fns(req).await
}
