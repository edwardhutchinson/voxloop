//! Authorisation — evaluates one requirement against a caller and answers permitted or
//! refused.
//!
//! Nothing here says *why* in a form the caller can act on. Transport turns a refusal into
//! something a human reads; this module answers the question and nothing more.
//!
//! Every operation carries exactly one [`Requirement`], typed at the point it is registered
//! ([ADR-0054]). There is no default value and no way to register an operation without one,
//! so an operation nobody ruled on is a build failure rather than an open door.
//!
//! [ADR-0054]: ../../docs/adr/0054-every-operation-declares-its-authorisation.md

/// What an operation demands of whoever calls it.
///
/// ADR-0054 fixes six requirements and no seventh. Five of them are a function of the caller
/// alone and are named here. The sixth, `Grid(rung, loop)`, is a function of the operation's
/// *arguments* as well — it names a loop the caller has yet to supply — and every operation
/// carrying it is a signalling-channel message rather than an HTTP route
/// (`docs/spec/api-surface.md`). It arrives with the socket and the grid, alongside the loop
/// identity and the four rungs it needs, and it is deliberately not invented here.
// Four of the five name a principal, and no principal exists until #30. They are declared
// together anyway: the list is fixed by ADR-0054, not grown one route at a time.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Requirement {
    /// No principal at all: the client bundle, sign-in, redemption, liveness.
    Public,
    /// An authenticated user who has assumed no role.
    SignedIn,
    /// A user who has assumed a role.
    Session,
    /// The user-level flag of ADR-0003, held by the person and never by a role.
    SystemAdministration,
    /// A service principal, presenting its token in an `Authorization` header.
    ServiceToken,
}

/// The answer, and the whole of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Permitted,
    Refused,
}

/// Decide whether this call may proceed.
///
/// Only `Public` can be satisfied today. The other four name a principal, and no principal
/// can be resolved until Identity exists (#30), so they are refused rather than waved
/// through — the default is refusal, everywhere and always.
pub(crate) fn evaluate(requirement: &Requirement) -> Outcome {
    match requirement {
        Requirement::Public => Outcome::Permitted,
        Requirement::SignedIn
        | Requirement::Session
        | Requirement::SystemAdministration
        | Requirement::ServiceToken => Outcome::Refused,
    }
}
