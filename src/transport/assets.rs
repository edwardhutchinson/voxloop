//! The client bundle, embedded at release ([ADR-0037]).
//!
//! SvelteKit builds with `adapter-static` and the release build embeds the result, so no
//! Node runtime is deployed. The embed sits behind the `embed-web` Cargo feature, off by
//! default: a bare `cargo build` never needs `web/dist`, and `web/dist` is never committed,
//! because Vite content-hashes its output and committed output both grows without bound and
//! goes quietly stale.
//!
//! A release build therefore has an ordering requirement: `npm run build` before
//! `cargo build --release --features embed-web`. A release build with the feature and no
//! `web/dist` fails outright; one built over a *stale* `web/dist` succeeds and ships the
//! previous console, which is the failure worth guarding in CI.
//!
//! [ADR-0037]: ../../../docs/adr/0037-the-client-ships-as-static-assets-embedded-at-release.md

use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};

/// Everything under here is an operation, not a page the console renders.
const OPERATIONS: &str = "/api/";

/// Serve the client bundle. Registered as the fallback, under `Requirement::Public`.
///
/// The console does its own routing, so an unknown path is ordinarily one of its pages. An
/// unknown *operation* is not: answering `/api/mistyped` with the console would let an
/// operation that does not exist look like one that does, which is the opposite of what
/// `docs/spec/api-surface.md` promises.
pub(super) async fn bundle(uri: Uri) -> Response {
    if uri.path().starts_with(OPERATIONS) {
        return (StatusCode::NOT_FOUND, "No such operation.").into_response();
    }

    page(uri).await
}

#[cfg(feature = "embed-web")]
async fn page(uri: Uri) -> Response {
    #[derive(rust_embed::Embed)]
    #[folder = "web/dist"]
    struct Bundle;

    let requested = uri.path().trim_start_matches('/');
    let requested = if requested.is_empty() {
        "index.html"
    } else {
        requested
    };

    // Anything the bundle does not hold is a route the client renders for itself.
    let Some(file) = Bundle::get(requested).or_else(|| Bundle::get("index.html")) else {
        return (StatusCode::NOT_FOUND, "Not found.").into_response();
    };

    let content_type = file.metadata.mimetype().to_owned();
    (
        [(header::CONTENT_TYPE, content_type)],
        file.data.into_owned(),
    )
        .into_response()
}

/// Stand in for the bundle in a build that did not embed it.
///
/// Development is two processes, Vite's dev server proxying to this binary, so this is the
/// ordinary state of a `cargo run` rather than a fault.
#[cfg(not(feature = "embed-web"))]
async fn page(_uri: Uri) -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        "This binary was built without the client bundle. Run the Vite dev server in web/ \
         for the console, or build a release: npm run build, then \
         cargo build --release --features embed-web.\n",
    )
        .into_response()
}
