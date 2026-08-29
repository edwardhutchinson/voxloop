//! Transport — HTTP routes, static assets, and (from #36) the signalling WebSocket.
//!
//! It sits at the top of the call graph: it receives requests, so nothing calls into it
//! ([ADR-0062]). It is also the only module that may name an HTTP status.
//!
//! TLS terminates here, in this binary, via rustls ([ADR-0040]). There is no reverse proxy:
//! a proxy would be a fifth moving part to install and patch inside a possibly air-gapped
//! network, and a second place a WebSocket upgrade can be misconfigured — on a system where
//! losing the signalling channel withdraws the emission path entirely.
//!
//! [ADR-0040]: ../../../docs/adr/0040-one-binary-one-unit-four-moving-parts.md
//! [ADR-0062]: ../../../docs/adr/0062-the-call-graph-is-acyclic-and-effects-modules-are-sinks.md

mod assets;
mod liveness;
mod routes;

use std::net::SocketAddr;
use std::time::Duration;

use axum_server::Handle;
use axum_server::tls_rustls::RustlsConfig;
use tokio::task::JoinHandle;

use crate::authorisation::Requirement;
use crate::configuration::Deployment;
use crate::telemetry::module;
use routes::RouteTable;

/// How long a connection has to finish what it was doing once the server is asked to stop.
const GRACE: Duration = Duration::from_secs(5);

/// A running server, and the handle that stops it.
pub(crate) struct Serving {
    handle: Handle<SocketAddr>,
    address: SocketAddr,
    task: JoinHandle<()>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum TransportError {
    #[error("the TLS certificate or key could not be read: {detail}")]
    Certificate { detail: String },

    #[error("nothing could listen on {address}")]
    CouldNotListen { address: SocketAddr },
}

/// Choose the cryptography rustls uses, rather than letting it be inferred.
///
/// More than one provider ends up in the dependency tree — the HTTP client used in tests
/// brings its own — and rustls refuses to guess between them. Naming one here means the
/// choice is a decision in the diff rather than a consequence of feature unification.
fn install_cryptography() {
    static CHOSEN: std::sync::Once = std::sync::Once::new();
    CHOSEN.call_once(|| {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

/// Start serving, and return once the listener is up.
pub(crate) async fn start(deployment: &Deployment) -> Result<Serving, TransportError> {
    install_cryptography();

    let tls = RustlsConfig::from_pem_file(&deployment.tls.certificate, &deployment.tls.private_key)
        .await
        .map_err(|error| TransportError::Certificate {
            detail: error.to_string(),
        })?;

    let handle = Handle::new();
    let asked_for = deployment.listen.address;
    let task = tokio::spawn({
        let handle = handle.clone();
        let routes = routes().into_make_service();
        async move {
            if let Err(error) = axum_server::bind_rustls(asked_for, tls)
                .handle(handle)
                .serve(routes)
                .await
            {
                tracing::error!(target: module::TRANSPORT, %error, "the server stopped");
            }
        }
    });

    let address = handle
        .listening()
        .await
        .ok_or(TransportError::CouldNotListen { address: asked_for })?;

    Ok(Serving {
        handle,
        address,
        task,
    })
}

impl Serving {
    /// Where the server actually ended up listening.
    pub(crate) fn address(&self) -> SocketAddr {
        self.address
    }

    /// Stop serving, giving open connections [`GRACE`] to finish.
    ///
    /// A restart is indistinguishable from total network loss to every client, so this is
    /// as gentle as it can afford to be and no gentler.
    pub(crate) async fn stop(self) {
        tracing::info!(target: module::TRANSPORT, "stopping");
        self.handle.graceful_shutdown(Some(GRACE));
        let _ = self.task.await;
    }
}

/// Every route the binary answers, each carrying the requirement it was registered under.
///
/// This function is the whole of what VoxLoop exposes. Reading it top to bottom is meant to
/// be the same experience as reading `docs/spec/api-surface.md`.
fn routes() -> RouteTable {
    RouteTable::new()
        .get("/api/liveness", Requirement::Public, liveness::liveness)
        .fallback(Requirement::Public, assets::bundle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// A deployment serving on a port the operating system picks, read from a real file, with
    /// a certificate made for the occasion. Nothing here is committed and nothing outlives
    /// the test.
    fn a_deployment_in(directory: &Path) -> Deployment {
        let issued = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()])
            .expect("a certificate for localhost");
        let certificate = directory.join("certificate.pem");
        let private_key = directory.join("private-key.pem");
        std::fs::write(&certificate, issued.cert.pem()).expect("the certificate to be written");
        std::fs::write(&private_key, issued.signing_key.serialize_pem())
            .expect("the key to be written");

        let file = directory.join("voxloop.toml");
        std::fs::write(
            &file,
            format!(
                "[listen]\naddress = \"127.0.0.1:0\"\n\n\
                 [tls]\ncertificate = \"{}\"\nprivate_key = \"{}\"\n\n\
                 [store]\npath = \"{}\"\n\n\
                 [log]\nlevel = \"warn\"\n",
                certificate.display(),
                private_key.display(),
                directory.join("voxloop.sqlite").display(),
            ),
        )
        .expect("the deployment file to be written");

        Deployment::load(&file).expect("a deployment")
    }

    /// A server, a client that trusts its certificate, and the liveness URL to ask for.
    async fn a_server_in(directory: &Path) -> (Serving, reqwest::Client, String) {
        let deployment = a_deployment_in(directory);
        let serving = start(&deployment).await.expect("the server to start");

        let root = reqwest::Certificate::from_pem(
            &std::fs::read(&deployment.tls.certificate).expect("the certificate to be read"),
        )
        .expect("a usable certificate");
        let client = reqwest::Client::builder()
            .add_root_certificate(root)
            .resolve("localhost", serving.address())
            .build()
            .expect("a client");

        let liveness = format!(
            "https://localhost:{}/api/liveness",
            serving.address().port()
        );

        (serving, client, liveness)
    }

    #[cfg(feature = "embed-web")]
    #[tokio::test]
    async fn serves_the_embedded_client_bundle_at_the_root() {
        let (status, body) = routes().answer_to("/").await;

        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(
            body.to_lowercase().contains("<!doctype html>"),
            "expected the client bundle's index, got {body:?}"
        );
    }

    #[cfg(not(feature = "embed-web"))]
    #[tokio::test]
    async fn says_so_plainly_when_the_client_bundle_was_not_embedded() {
        let (status, body) = routes().answer_to("/").await;

        assert_eq!(status, axum::http::StatusCode::NOT_FOUND);
        assert!(
            body.contains("npm run build"),
            "expected the answer to say how to get a bundle, got {body:?}"
        );
    }

    #[tokio::test]
    async fn an_operation_that_does_not_exist_is_not_answered_with_the_console() {
        let (status, body) = routes().answer_to("/api/no-such-operation").await;

        assert_eq!(status, axum::http::StatusCode::NOT_FOUND);
        assert!(
            !body.to_lowercase().contains("<!doctype html>"),
            "expected a refusal rather than the console, got {body:?}"
        );
    }

    #[tokio::test]
    async fn answers_liveness_over_tls_and_says_nothing_else() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let (serving, client, liveness) = a_server_in(directory.path()).await;

        let answer = client
            .get(&liveness)
            .send()
            .await
            .expect("an answer over TLS");

        assert_eq!(answer.status(), reqwest::StatusCode::OK);
        assert_eq!(answer.text().await.expect("a body"), "alive");
        serving.stop().await;
    }

    #[tokio::test]
    async fn stops_answering_once_it_is_asked_to_stop() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let (serving, client, liveness) = a_server_in(directory.path()).await;
        client.get(&liveness).send().await.expect("an answer");

        serving.stop().await;

        assert!(
            client.get(&liveness).send().await.is_err(),
            "expected the listener to be gone once the server had stopped"
        );
    }
}
