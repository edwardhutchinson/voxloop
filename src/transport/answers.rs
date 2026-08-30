//! Every HTTP status VoxLoop names, in the one module allowed to name one.
//!
//! **A refusal says what the caller did not meet** (v1 §3), rather than hiding an operation's
//! existence. There is one organisation on one box; pretending an operation is not there
//! tells an operator with a stale tab that the product is broken. The bootstrap route is the
//! sole exception, and it genuinely stops existing rather than answering evasively.
//!
//! **A credential that was not accepted is not a refusal**, and does not answer as one. The
//! caller may perform the operation; what they presented was wrong. Telling somebody holding
//! a mistyped enrolment code that they may not redeem enrolment codes is an answer they
//! cannot act on, so those routes answer [`not_accepted`] instead.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::configuration::StoreError;
use crate::telemetry::module;

/// The caller may not, and here is what they did not meet.
///
/// The reason is the whole of the answer. Prefixing it with *You may not* was tried and
/// dropped: the status already says refused, in the one module entitled to say it, and the
/// prefix read as a non-sequitur on every message whose reason is a fact about the
/// deployment rather than about the caller's standing.
pub(super) fn refusal(reason: &str) -> Response {
    (StatusCode::FORBIDDEN, format!("{reason}\n")).into_response()
}

/// Something the caller presented was not accepted.
///
/// Not a refusal: they may perform this operation, and the credential they offered is not
/// one this deployment holds. A password, a bootstrap code and an enrolment code all answer
/// this way, and none of them says which part was wrong — whether a username exists, or
/// whether a code was never issued rather than already spent, is not something an
/// unauthenticated caller is entitled to learn.
pub(super) fn not_accepted(reason: &str) -> Response {
    (StatusCode::UNAUTHORIZED, format!("{reason}\n")).into_response()
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

/// There is no such record.
///
/// Not a refusal, and not a hidden operation: the caller may read users, and this is VoxLoop
/// saying it holds no user by that id.
pub(super) fn no_such(what: &str) -> Response {
    (StatusCode::NOT_FOUND, format!("There is no such {what}.\n")).into_response()
}

/// Answer, unless the store could not answer at all.
///
/// Every handler ends this way, so the one path that turns a store fault into a status
/// exists once rather than per route.
pub(super) fn or_unavailable(answer: Result<Response, StoreError>) -> Response {
    match answer {
        Ok(answer) => answer,
        Err(error) => unavailable(&error),
    }
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
