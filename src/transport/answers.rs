//! Every HTTP status VoxLoop names, in the one module allowed to name one.
//!
//! **A refusal says *you may not*, with the reason** (v1 §3), rather than hiding an
//! operation's existence. There is one organisation on one box; pretending an operation is
//! not there tells an operator with a stale tab that the product is broken. The bootstrap
//! route is the sole exception, and it genuinely stops existing rather than answering
//! evasively.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::configuration::StoreError;
use crate::telemetry::module;

/// You may not, and here is why.
pub(super) fn refusal(reason: &str) -> Response {
    (StatusCode::FORBIDDEN, format!("You may not. {reason}\n")).into_response()
}

/// The credentials presented were not accepted.
///
/// It does not say which half was wrong. Whether a username exists is not something an
/// unauthenticated caller is entitled to learn.
pub(super) fn credentials_refused() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        "You may not. Those credentials were not accepted.\n",
    )
        .into_response()
}

/// Too many attempts from here, or too many across the deployment.
///
/// Limits key on **source**, never on the submitted account name, so this throttles a
/// machine and never an account: no number of failures locks anybody out ([ADR-0025]).
///
/// [ADR-0025]: ../../../docs/adr/0025-credentials-are-administered-because-there-is-no-email.md
pub(super) fn too_many_attempts() -> Response {
    (
        StatusCode::TOO_MANY_REQUESTS,
        "Too many attempts. Wait a moment and try again.\n",
    )
        .into_response()
}

/// What was asked for cannot be done as it was asked.
///
/// Not a refusal: the caller may perform this operation, and this particular attempt at it
/// will not do.
pub(super) fn cannot(reason: &str) -> Response {
    (StatusCode::BAD_REQUEST, format!("{reason}\n")).into_response()
}

/// Something VoxLoop needed was not there. Said without naming what it was.
pub(super) fn unavailable(error: &StoreError) -> Response {
    tracing::error!(target: module::TRANSPORT, %error, "the request could not be answered");

    (
        StatusCode::SERVICE_UNAVAILABLE,
        "VoxLoop could not answer that just now.\n",
    )
        .into_response()
}
