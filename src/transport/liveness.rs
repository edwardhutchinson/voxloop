//! The one route a customer's monitoring may poll without signing in.

/// Answer that the process is alive, and answer nothing else.
///
/// No version, no counts, no user or loop names ([ADR-0054]). Subprocess, disk and backup
/// health sit on the admin console behind the system-administration flag, because that is
/// the capability entitled to see them.
///
/// [ADR-0054]: ../../../docs/adr/0054-every-operation-declares-its-authorisation.md
pub(super) async fn liveness() -> &'static str {
    "alive"
}
