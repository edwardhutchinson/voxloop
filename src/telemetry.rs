//! The logging facade, and the span naming scheme every module logs through.
//!
//! `tracing` was chosen early so that observability is a wiring job rather than a retrofit
//! ([ADR-0036]). The scheme is keyed to the eleven modules of [`docs/spec/modules.md`]: a
//! span's **target is the module it belongs to** and its **name is the domain operation**,
//! so `sign_in` opened in Identity reads as `voxloop::identity` / `sign_in` and never has to
//! be traced back to a file to find out which part of the system emitted it.
//!
//! Filtering is therefore per module: `VOXLOOP_LOG__LEVEL="info,voxloop::media_plane=debug"`
//! turns up one module without turning up the rest.
//!
//! This is the logging facade rather than a module in the ADR-0060 sense: it has no domain
//! interface and every module writes through it.
//!
//! [ADR-0036]: ../../docs/adr/0036-the-backend-is-rust-on-axum.md
//! [`docs/spec/modules.md`]: ../../docs/spec/modules.md

use tracing_subscriber::EnvFilter;

/// One target per module in the binary. The other four modules of `docs/spec/modules.md` are
/// in the client bundle, which has no logging of its own yet; when it gains some, these are
/// the names it takes.
///
/// All seven are declared together because the scheme is the point: a module that invents its
/// own target when it lands is a module whose logs nobody can filter for.
#[allow(dead_code)] // Four of the seven modules arrive in later tickets.
pub(crate) mod module {
    pub(crate) const TRANSPORT: &str = "voxloop::transport";
    pub(crate) const AUTHORISATION: &str = "voxloop::authorisation";
    pub(crate) const IDENTITY: &str = "voxloop::identity";
    pub(crate) const CONFIGURATION: &str = "voxloop::configuration";
    pub(crate) const STATE_AUTHORITY: &str = "voxloop::state_authority";
    pub(crate) const MEDIA_PLANE: &str = "voxloop::media_plane";
    pub(crate) const SYNTHESIS: &str = "voxloop::synthesis";
}

#[derive(Debug, thiserror::Error)]
#[error("the log level {level:?} is not one this binary understands: {detail}")]
pub(crate) struct TelemetryError {
    level: String,
    detail: String,
}

/// Start logging at the level the deployment file asks for.
///
/// The level is a filter directive, so it takes either a bare level or a per-module list.
pub(crate) fn start(level: &str) -> Result<(), TelemetryError> {
    let filter = EnvFilter::try_new(level).map_err(|error| TelemetryError {
        level: level.to_owned(),
        detail: error.to_string(),
    })?;

    tracing_subscriber::fmt().with_env_filter(filter).init();

    Ok(())
}
