//! VoxLoop — the one binary a deployment runs.
//!
//! A deployment is four moving parts: this binary (console, API, signalling, permission
//! enforcement, TLS), the mediasoup C++ worker, the text-to-speech sidecar, and one SQLite
//! file ([ADR-0040]). This file is the composition root and nothing else: it reads the
//! deployment file, starts logging, opens the store, serves, and stops when it is told to.
//!
//! The modules under it are the seams ([`docs/spec/modules.md`], [ADR-0060]). Each makes
//! exactly its interface `pub(crate)` and keeps the rest private, which is what makes Rust's
//! own privacy the enforcement rather than a review convention ([ADR-0061]). Widening
//! `pub(crate)` is the cheapest edit in this codebase and it is how a seam quietly stops
//! existing.
//!
//! [ADR-0040]: ../../docs/adr/0040-one-binary-one-unit-four-moving-parts.md
//! [ADR-0060]: ../../docs/adr/0060-a-seam-names-domain-operations.md
//! [ADR-0061]: ../../docs/adr/0061-module-privacy-is-the-seam-enforcement.md
//! [`docs/spec/modules.md`]: ../../docs/spec/modules.md

// ADR-0037 puts the embed "off by default and on for release". Cargo has no per-profile
// feature, so the second half is this: a release build that would ship without the console
// does not compile. Building a release is therefore one command, and it is the right one.
#[cfg(all(not(debug_assertions), not(feature = "embed-web")))]
compile_error!(
    "a release build must carry the console: run `npm run build` in web/, then \
     `cargo build --release --features embed-web`"
);

mod authorisation;
mod configuration;
mod identity;
mod lifetimes;
mod media_plane;
mod on_box;
mod secrets;
mod state;
mod supervision;
mod telemetry;
mod transport;

use std::path::Path;
use std::process::ExitCode;

use std::sync::Arc;

use configuration::{Deployment, DeploymentError, Store, StoreError};
use identity::{Bootstrap, Identity};
use media_plane::{MediaPlane, MediaPlaneError};
use on_box::{Invocation, OnBoxError};
use state::StateAuthority;
use telemetry::{TelemetryError, module};
use transport::TransportError;

/// Where the deployment file is when nobody says otherwise.
const DEPLOYMENT_FILE: &str = "voxloop.toml";

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(refusal) => {
            // Startup can fail before there is anywhere to log to, so this goes to stderr
            // whatever happens. systemd will have it either way.
            eprintln!("voxloop: {refusal}");
            ExitCode::FAILURE
        }
    }
}

/// Everything that stops VoxLoop.
///
/// Almost all of it is startup: a deployment file that is not there, a certificate that
/// cannot be read, a store that will not open, a worker that will not start. The one
/// exception is [`Fatal::TheAudioStopped`], which happens long afterwards and is here for the
/// same reason as the rest — it is a reason this process is over, and the operator gets it on
/// stderr and systemd gets it as an exit code.
#[derive(Debug, thiserror::Error)]
enum Fatal {
    #[error(transparent)]
    Deployment(#[from] DeploymentError),

    #[error(transparent)]
    Telemetry(#[from] TelemetryError),

    #[error(transparent)]
    Store(#[from] StoreError),

    #[error(transparent)]
    MediaPlane(#[from] MediaPlaneError),

    #[error("the mediasoup worker stopped, so nothing was carrying audio any more")]
    TheAudioStopped,

    #[error(transparent)]
    Transport(#[from] TransportError),

    #[error(transparent)]
    OnBox(#[from] OnBoxError),
}

/// Serve, or do one of the two things the on-box CLI does and stop.
///
/// The CLI is deliberately the same binary. A separate one would be a second artefact to
/// ship, keep in step with the schema, and find on a box at the moment somebody is locked
/// out of the deployment — which is the only moment it is ever run ([ADR-0025]).
///
/// [ADR-0025]: ../../docs/adr/0025-credentials-are-administered-because-there-is-no-email.md
async fn run() -> Result<(), Fatal> {
    match on_box::invoked(std::env::args().skip(1))? {
        Invocation::Serve { deployment } => serve(&deployment).await,
        Invocation::MakeAnAdministrator {
            deployment,
            username,
        } => {
            on_box::make_an_administrator(&on_the_box(&deployment).await?, &username)
                .await?
                .say();
            Ok(())
        }
        Invocation::ResetAPassword {
            deployment,
            username,
        } => {
            on_box::reset_a_password(&on_the_box(&deployment).await?, &username)
                .await?
                .say();
            Ok(())
        }
        Invocation::Explain => {
            on_box::explain();
            Ok(())
        }
    }
}

/// Open the store a CLI command acts on, and nothing else.
///
/// No telemetry subscriber is started: the operator ran a command and is owed its answer on
/// stdout, not the deployment's configured log level poured over the top of it.
async fn on_the_box(deployment: &Path) -> Result<Store, Fatal> {
    let deployment = Deployment::load(deployment)?;

    Ok(Store::open(&deployment.store.path).await?)
}

async fn serve(deployment: &Path) -> Result<(), Fatal> {
    let deployment = Deployment::load(deployment)?;
    telemetry::start(&deployment.log.level)?;

    let store = Arc::new(Store::open(&deployment.store.path).await?);

    // No default credentials, ever: a deployment nobody administers yet mints a one-time
    // code to this log, and one that somebody does mints nothing and registers no route.
    let bootstrap = Bootstrap::mint_unless_administered(&store).await?;

    // Live state starts empty, and that is the honest state of a box that has just come up:
    // a restart ends every session, because the media plane cannot survive one and occupancy
    // restored without an audio path would be a lie (ADR-0039). Sign-ins are durable and
    // survive, so everybody who was on console is signed in, in the lobby.
    let state = Arc::new(StateAuthority::empty());

    // One Worker, one Router and one port, before anything is served. A deployment that
    // cannot carry audio has lost its whole purpose, so it refuses to start rather than
    // offering a console that will never make a sound (ADR-0006).
    let (media, reports, carriageway) = MediaPlane::carrying(&deployment.media).await?;

    // The media plane calls nothing and reports on a channel (ADR-0062), so this is what
    // turns what it says into live state. It starts before the first session can exist.
    let watching = supervision::watching(reports, Arc::clone(&state));

    let serving = transport::start(
        &deployment,
        Arc::clone(&store),
        Arc::clone(&state),
        Identity::local_passwords(),
        media,
        bootstrap,
    )
    .await?;
    tracing::info!(target: module::TRANSPORT, address = %serving.address(), "serving");

    // The one clock VoxLoop runs: a sign-in ends after 24 hours with no deliberate act, and
    // only while it stands in the lobby (v1 §2).
    let sweeping = lifetimes::sweeping(Arc::clone(&store), state);

    // Two things stop VoxLoop. One is somebody asking. The other is the worker dying, which
    // takes every transport with it and cannot be recovered in place ([ADR-0070]) — so the
    // honest end is the one every deployment already exercises: go down, let systemd bring
    // the unit back, and end every session rather than serving consoles that will never
    // make a sound again.
    let carrying_audio = tokio::select! {
        () = wait_to_be_stopped() => true,
        () = watching.until_nothing_is_carried() => false,
    };

    sweeping.stop();

    // A restart is indistinguishable from total network loss to every client, and it ends
    // every session, so the least this can do is put the store down cleanly.
    //
    // The order is sockets, then reports, then audio, then the store: a console still being
    // answered may yet be told something, a report still in flight is still worth writing
    // down, and nothing is owed to a port nobody is listening on.
    serving.stop().await;
    watching.stop();
    carriageway.stop();
    store.close().await;
    tracing::info!(target: module::TRANSPORT, "stopped");

    // A deliberate stop is a success and a worker's death is not, so the exit code tells
    // them apart: systemd restarts what failed, and a unit that came down on purpose stays
    // down.
    match carrying_audio {
        true => Ok(()),
        false => Err(Fatal::TheAudioStopped),
    }
}

/// Wait for systemd to stop the unit, or for someone at a terminal to interrupt it.
async fn wait_to_be_stopped() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        let mut terminate = match signal(SignalKind::terminate()) {
            Ok(terminate) => terminate,
            Err(error) => {
                tracing::warn!(target: module::TRANSPORT, %error, "no SIGTERM handler");
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };

        tokio::select! {
            _ = terminate.recv() => {}
            _ = tokio::signal::ctrl_c() => {}
        }
    }

    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}
