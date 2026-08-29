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

mod answers;
mod assets;
mod bootstrap;
mod cookies;
mod entry;
mod liveness;
mod rate_limit;
mod routes;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum_server::Handle;
use axum_server::tls_rustls::RustlsConfig;
use tokio::task::JoinHandle;

use crate::authorisation::Requirement;
use crate::configuration::{Deployment, Store};
use crate::identity::{Bootstrap, Identity};
use crate::telemetry::module;
use rate_limit::RateLimits;
use routes::RouteTable;

/// Everything a handler is given, and nothing it is not.
///
/// Transport may call Authorisation, Configuration, Identity, the state authority and
/// Synthesis ([`docs/spec/modules.md`]), and what it holds of each of them is here. A handler
/// reaches nothing that is not in this struct.
///
/// [`docs/spec/modules.md`]: ../../../docs/spec/modules.md
#[derive(Clone)]
pub(super) struct Api {
    store: Arc<Store>,
    identity: Identity,
    limits: Arc<RateLimits>,
    /// The code this run of the process minted, where nobody administers this deployment
    /// yet. `None` is the ordinary state, and it is what leaves the redemption route
    /// unregistered rather than merely refusing.
    bootstrap: Option<Arc<Bootstrap>>,
}

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
pub(crate) async fn start(
    deployment: &Deployment,
    store: Arc<Store>,
    identity: Identity,
    bootstrap: Option<Bootstrap>,
) -> Result<Serving, TransportError> {
    install_cryptography();

    let api = Api {
        store,
        identity,
        limits: Arc::new(RateLimits::default()),
        bootstrap: bootstrap.map(Arc::new),
    };

    let tls = RustlsConfig::from_pem_file(&deployment.tls.certificate, &deployment.tls.private_key)
        .await
        .map_err(|error| TransportError::Certificate {
            detail: error.to_string(),
        })?;

    let handle = Handle::new();
    let asked_for = deployment.listen.address;
    let task = tokio::spawn({
        let handle = handle.clone();
        let routes = routes(&api).into_make_service(api.clone());
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
fn routes(api: &Api) -> RouteTable<Api> {
    let mut table = RouteTable::new(Arc::clone(&api.store))
        .get("/api/liveness", Requirement::Public, liveness::liveness)
        .post("/api/sign-in", Requirement::Public, entry::sign_in)
        .post("/api/sign-out", Requirement::SignedIn, entry::sign_out);

    // Registered only while no system administrator exists, and genuinely absent otherwise:
    // the one operation VoxLoop hides rather than refuses (v1 §3).
    if api.bootstrap.is_some() {
        table = table.post("/api/bootstrap", Requirement::Public, bootstrap::redeem);
    }

    table.fallback(Requirement::Public, assets::bundle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    use axum::body::Body;
    use axum::extract::ConnectInfo;
    use axum::http::{Request, StatusCode, header};
    use axum::response::Response;

    use crate::configuration::{AuditEvent, AuditLog, NewUser, Users, a_temporary_store};

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

    /// A box: one store, one identity, and whatever the last start made of the bootstrap
    /// code. Requests go through the whole route table, so what is exercised is what the
    /// server serves.
    struct ABox {
        _directory: tempfile::TempDir,
        api: Api,
    }

    impl ABox {
        /// A fresh box, with nobody on it.
        async fn with_nobody_on_it() -> Self {
            let (directory, store) = a_temporary_store().await;
            let store = Arc::new(store);
            let bootstrap = Bootstrap::mint_unless_administered(&store)
                .await
                .expect("the store to answer");

            Self {
                _directory: directory,
                api: Api {
                    store,
                    identity: Identity::local_passwords(),
                    limits: Arc::new(RateLimits::default()),
                    bootstrap: bootstrap.map(Arc::new),
                },
            }
        }

        /// A box somebody already administers, started the way a real one would be.
        async fn already_administered() -> Self {
            let mut box_of = Self::with_nobody_on_it().await;
            let identity = box_of.api.identity.clone();
            let mut transaction = box_of.api.store.begin().await.expect("a transaction");
            transaction
                .create_user(NewUser {
                    username: "root".to_owned(),
                    password_hash: Some(
                        identity
                            .hash_password("an established password")
                            .expect("the password to hash"),
                    ),
                    is_system_administrator: true,
                })
                .await
                .expect("an administrator");
            transaction
                .commit()
                .await
                .expect("the administrator to land");

            box_of.api.bootstrap = Bootstrap::mint_unless_administered(&box_of.api.store)
                .await
                .expect("the store to answer")
                .map(Arc::new);

            box_of
        }

        fn bootstrap_code(&self) -> String {
            self.api
                .bootstrap
                .as_ref()
                .expect("a bootstrap code")
                .code()
                .expect("an unspent code")
        }

        /// Ask for something, from a named source, presenting whatever cookie is held.
        async fn ask(&self, request: Request<Body>) -> Answer {
            let answer = routes(&self.api).answer(self.api.clone(), request).await;

            Answer::of(answer).await
        }

        async fn get(&self, path: &str) -> Answer {
            self.ask(from("192.0.2.1").uri(path).body(Body::empty()).unwrap())
                .await
        }

        async fn post(&self, path: &str, body: &str) -> Answer {
            self.post_from("192.0.2.1", path, body).await
        }

        async fn post_from(&self, source: &str, path: &str, body: &str) -> Answer {
            self.ask(
                from(source)
                    .method("POST")
                    .uri(path)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_owned()))
                    .unwrap(),
            )
            .await
        }

        async fn post_holding(&self, cookie: &str, path: &str, body: &str) -> Answer {
            self.ask(
                from("192.0.2.1")
                    .method("POST")
                    .uri(path)
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::COOKIE, cookie)
                    .body(Body::from(body.to_owned()))
                    .unwrap(),
            )
            .await
        }

        /// Everything the audit log holds, newest first.
        async fn audited(&self) -> Vec<(AuditEvent, String, Option<String>)> {
            let mut transaction = self.api.store.begin().await.expect("a transaction");
            let entries = transaction
                .recent_entries(20)
                .await
                .expect("the log to be readable");

            entries
                .into_iter()
                .map(|entry| (entry.event, entry.actor_name, entry.source))
                .collect()
        }
    }

    /// A request from a source, the way the server would have one.
    fn from(source: &str) -> axum::http::request::Builder {
        Request::builder().extension(ConnectInfo(SocketAddr::new(
            source.parse().expect("an address"),
            51_000,
        )))
    }

    /// What came back, with the parts a test wants to read already out of it.
    struct Answer {
        status: StatusCode,
        cookie: Option<String>,
        body: String,
    }

    impl Answer {
        async fn of(response: Response) -> Self {
            let status = response.status();
            let cookie = response
                .headers()
                .get(header::SET_COOKIE)
                .map(|value| value.to_str().expect("a readable cookie").to_owned());
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("a body");

            Self {
                status,
                cookie,
                body: String::from_utf8_lossy(&body).into_owned(),
            }
        }

        /// The cookie as a browser would present it back.
        fn presented(&self) -> String {
            self.cookie
                .as_ref()
                .expect("a cookie")
                .split(';')
                .next()
                .expect("a name and value")
                .to_owned()
        }
    }

    fn redeeming(code: &str, username: &str, password: &str) -> String {
        format!(r#"{{"code":"{code}","username":"{username}","password":"{password}"}}"#)
    }

    fn signing_in(username: &str, password: &str) -> String {
        format!(r#"{{"username":"{username}","password":"{password}"}}"#)
    }

    #[cfg(feature = "embed-web")]
    #[tokio::test]
    async fn serves_the_embedded_client_bundle_at_the_root() {
        let box_of = ABox::with_nobody_on_it().await;

        let answer = box_of.get("/").await;

        assert_eq!(answer.status, StatusCode::OK);
        assert!(
            answer.body.to_lowercase().contains("<!doctype html>"),
            "expected the client bundle's index, got {:?}",
            answer.body
        );
    }

    #[cfg(not(feature = "embed-web"))]
    #[tokio::test]
    async fn says_so_plainly_when_the_client_bundle_was_not_embedded() {
        let box_of = ABox::with_nobody_on_it().await;

        let answer = box_of.get("/").await;

        assert_eq!(answer.status, StatusCode::NOT_FOUND);
        assert!(
            answer.body.contains("npm run build"),
            "expected the answer to say how to get a bundle, got {:?}",
            answer.body
        );
    }

    #[tokio::test]
    async fn an_operation_that_does_not_exist_is_not_answered_with_the_console() {
        let box_of = ABox::with_nobody_on_it().await;

        let answer = box_of.get("/api/no-such-operation").await;

        assert_eq!(answer.status, StatusCode::NOT_FOUND);
        assert!(
            !answer.body.to_lowercase().contains("<!doctype html>"),
            "expected a refusal rather than the console, got {:?}",
            answer.body
        );
    }

    /// The whole of ticket #30 in one path: a box with nobody on it becomes a box with a
    /// system administrator, who signs in and signs out again.
    #[tokio::test]
    async fn a_fresh_box_becomes_one_with_an_administrator_who_signs_in_and_out() {
        let box_of = ABox::with_nobody_on_it().await;
        let code = box_of.bootstrap_code();

        let redeemed = box_of
            .post(
                "/api/bootstrap",
                &redeeming(&code, "flight", "a long enough password"),
            )
            .await;
        assert_eq!(redeemed.status, StatusCode::NO_CONTENT);

        let signed_in = box_of
            .post(
                "/api/sign-in",
                &signing_in("flight", "a long enough password"),
            )
            .await;
        assert_eq!(signed_in.status, StatusCode::NO_CONTENT);

        let signed_out = box_of
            .post_holding(&signed_in.presented(), "/api/sign-out", "")
            .await;
        assert_eq!(signed_out.status, StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn the_bootstrap_route_does_not_exist_once_an_administrator_does() {
        let box_of = ABox::already_administered().await;

        let answer = box_of
            .post(
                "/api/bootstrap",
                &redeeming("any code at all", "second", "a long enough password"),
            )
            .await;

        assert_eq!(answer.status, StatusCode::NOT_FOUND);
        assert_eq!(answer.body, "No such operation.");
    }

    #[tokio::test]
    async fn the_bootstrap_code_is_good_once() {
        let box_of = ABox::with_nobody_on_it().await;
        let code = box_of.bootstrap_code();
        box_of
            .post(
                "/api/bootstrap",
                &redeeming(&code, "flight", "a long enough password"),
            )
            .await;

        let again = box_of
            .post(
                "/api/bootstrap",
                &redeeming(&code, "second", "a long enough password"),
            )
            .await;

        assert_eq!(again.status, StatusCode::FORBIDDEN);
        assert!(again.body.starts_with("You may not."), "{:?}", again.body);
    }

    #[tokio::test]
    async fn a_code_nobody_minted_is_refused_and_does_not_spend_the_one_that_was() {
        let box_of = ABox::with_nobody_on_it().await;
        let code = box_of.bootstrap_code();

        let guessed = box_of
            .post(
                "/api/bootstrap",
                &redeeming("not the code", "flight", "a long enough password"),
            )
            .await;
        assert_eq!(guessed.status, StatusCode::FORBIDDEN);

        let redeemed = box_of
            .post(
                "/api/bootstrap",
                &redeeming(&code, "flight", "a long enough password"),
            )
            .await;
        assert_eq!(redeemed.status, StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn a_password_under_the_floor_is_refused_and_does_not_spend_the_code() {
        let box_of = ABox::with_nobody_on_it().await;
        let code = box_of.bootstrap_code();

        let refused = box_of
            .post("/api/bootstrap", &redeeming(&code, "flight", "too short"))
            .await;
        assert_eq!(refused.status, StatusCode::BAD_REQUEST);
        assert!(refused.body.contains("12"), "{:?}", refused.body);

        let redeemed = box_of
            .post(
                "/api/bootstrap",
                &redeeming(&code, "flight", "a long enough password"),
            )
            .await;
        assert_eq!(redeemed.status, StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn redeeming_the_code_is_audited() {
        let box_of = ABox::with_nobody_on_it().await;
        let code = box_of.bootstrap_code();

        box_of
            .post(
                "/api/bootstrap",
                &redeeming(&code, "flight", "a long enough password"),
            )
            .await;

        assert_eq!(
            box_of.audited().await,
            [(
                AuditEvent::BootstrapRedeemed,
                "flight".to_owned(),
                Some("192.0.2.1".to_owned())
            )]
        );
    }

    /// The cookie carries an opaque token and nothing else — not the username, not the
    /// system-administration flag the account it just made happens to hold (v1 §3).
    #[tokio::test]
    async fn the_cookie_is_secure_and_carries_no_claims() {
        let box_of = ABox::with_nobody_on_it().await;
        let code = box_of.bootstrap_code();
        box_of
            .post(
                "/api/bootstrap",
                &redeeming(&code, "flight", "a long enough password"),
            )
            .await;

        let signed_in = box_of
            .post(
                "/api/sign-in",
                &signing_in("flight", "a long enough password"),
            )
            .await;

        let cookie = signed_in.cookie.expect("a cookie");
        assert!(cookie.contains("Secure"), "{cookie}");
        assert!(cookie.contains("HttpOnly"), "{cookie}");
        assert!(!cookie.to_lowercase().contains("flight"), "{cookie}");
        assert!(!cookie.to_lowercase().contains("admin"), "{cookie}");
    }

    #[tokio::test]
    async fn a_signed_in_operation_is_refused_to_a_browser_holding_nothing() {
        let box_of = ABox::with_nobody_on_it().await;

        let answer = box_of.post("/api/sign-out", "").await;

        assert_eq!(answer.status, StatusCode::FORBIDDEN);
        assert!(answer.body.starts_with("You may not."), "{:?}", answer.body);
    }

    #[tokio::test]
    async fn signing_out_stops_the_cookie_working_at_once() {
        let box_of = ABox::with_nobody_on_it().await;
        let code = box_of.bootstrap_code();
        box_of
            .post(
                "/api/bootstrap",
                &redeeming(&code, "flight", "a long enough password"),
            )
            .await;
        let signed_in = box_of
            .post(
                "/api/sign-in",
                &signing_in("flight", "a long enough password"),
            )
            .await;
        let held = signed_in.presented();

        box_of.post_holding(&held, "/api/sign-out", "").await;

        let again = box_of.post_holding(&held, "/api/sign-out", "").await;
        assert_eq!(again.status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn signing_in_and_signing_out_are_both_audited() {
        let box_of = ABox::with_nobody_on_it().await;
        let code = box_of.bootstrap_code();
        box_of
            .post(
                "/api/bootstrap",
                &redeeming(&code, "flight", "a long enough password"),
            )
            .await;
        let signed_in = box_of
            .post(
                "/api/sign-in",
                &signing_in("flight", "a long enough password"),
            )
            .await;

        box_of
            .post_holding(&signed_in.presented(), "/api/sign-out", "")
            .await;

        let audited = box_of.audited().await;
        assert_eq!(
            audited[0],
            (AuditEvent::SignedOut, "flight".to_owned(), None)
        );
        assert_eq!(
            audited[1],
            (
                AuditEvent::SignInSucceeded,
                "flight".to_owned(),
                Some("192.0.2.1".to_owned())
            )
        );
    }

    #[tokio::test]
    async fn a_wrong_password_is_refused_and_recorded_against_the_source_it_came_from() {
        let box_of = ABox::with_nobody_on_it().await;
        let code = box_of.bootstrap_code();
        box_of
            .post(
                "/api/bootstrap",
                &redeeming(&code, "flight", "a long enough password"),
            )
            .await;

        let refused = box_of
            .post_from(
                "198.51.100.9",
                "/api/sign-in",
                &signing_in("flight", "the wrong password"),
            )
            .await;

        assert_eq!(refused.status, StatusCode::UNAUTHORIZED);
        assert!(refused.cookie.is_none(), "a refusal handed out a cookie");
        assert_eq!(
            box_of.audited().await[0],
            (
                AuditEvent::SignInFailed,
                "flight".to_owned(),
                Some("198.51.100.9".to_owned())
            )
        );
    }

    /// A name nobody holds is refused exactly as a wrong password is, and the log gets the
    /// name that was submitted rather than an actor there is none of.
    #[tokio::test]
    async fn a_name_nobody_holds_is_refused_the_same_way_a_wrong_password_is() {
        let box_of = ABox::with_nobody_on_it().await;

        let refused = box_of
            .post(
                "/api/sign-in",
                &signing_in("nobody-by-that-name", "a long enough password"),
            )
            .await;

        assert_eq!(refused.status, StatusCode::UNAUTHORIZED);
        assert_eq!(
            box_of.audited().await[0].1,
            "nobody-by-that-name".to_owned()
        );
    }

    /// Limits key on source, and no number of failures locks the account: the same
    /// credentials that were throttled from one machine are accepted from another.
    #[tokio::test]
    async fn attempts_are_throttled_by_source_and_no_number_of_them_locks_the_account() {
        let box_of = ABox::with_nobody_on_it().await;
        let code = box_of.bootstrap_code();
        box_of
            .post(
                "/api/bootstrap",
                &redeeming(&code, "flight", "a long enough password"),
            )
            .await;

        let mut throttled = None;
        for attempt in 0..40 {
            let answer = box_of
                .post_from(
                    "198.51.100.9",
                    "/api/sign-in",
                    &signing_in("flight", "the wrong password"),
                )
                .await;
            if answer.status == StatusCode::TOO_MANY_REQUESTS {
                throttled = Some(attempt);
                break;
            }
        }
        assert!(throttled.is_some(), "the attempts were never throttled");

        let elsewhere = box_of
            .post_from(
                "203.0.113.4",
                "/api/sign-in",
                &signing_in("flight", "a long enough password"),
            )
            .await;
        assert_eq!(
            elsewhere.status,
            StatusCode::NO_CONTENT,
            "the account was locked by failures from somewhere else"
        );
    }

    /// A server on a fresh store, a client that trusts its certificate, the URL it is
    /// serving on, and the bootstrap code it minted on the way up.
    async fn a_server_in(directory: &Path) -> (Serving, reqwest::Client, String, String) {
        let deployment = a_deployment_in(directory);
        let store = Arc::new(
            Store::open(&deployment.store.path)
                .await
                .expect("the store to open"),
        );
        let bootstrap = Bootstrap::mint_unless_administered(&store)
            .await
            .expect("the store to answer");
        let code = bootstrap
            .as_ref()
            .and_then(Bootstrap::code)
            .expect("a bootstrap code");
        let serving = start(&deployment, store, Identity::local_passwords(), bootstrap)
            .await
            .expect("the server to start");

        let root = reqwest::Certificate::from_pem(
            &std::fs::read(&deployment.tls.certificate).expect("the certificate to be read"),
        )
        .expect("a usable certificate");
        let client = reqwest::Client::builder()
            .add_root_certificate(root)
            .resolve("localhost", serving.address())
            .build()
            .expect("a client");

        let at = format!("https://localhost:{}", serving.address().port());

        (serving, client, at, code)
    }

    #[tokio::test]
    async fn answers_liveness_over_tls_and_says_nothing_else() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let (serving, client, at, _code) = a_server_in(directory.path()).await;

        let answer = client
            .get(format!("{at}/api/liveness"))
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
        let (serving, client, at, _code) = a_server_in(directory.path()).await;
        let liveness = format!("{at}/api/liveness");
        client.get(&liveness).send().await.expect("an answer");

        serving.stop().await;

        assert!(
            client.get(&liveness).send().await.is_err(),
            "expected the listener to be gone once the server had stopped"
        );
    }

    /// The same path as the in-process tests, over the wire a deployment actually serves —
    /// where a `Secure` cookie is the reason HTTPS is mandatory even on a LAN.
    #[tokio::test]
    async fn bootstraps_and_signs_a_browser_in_over_tls() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let (serving, client, at, code) = a_server_in(directory.path()).await;

        let redeemed = client
            .post(format!("{at}/api/bootstrap"))
            .header("content-type", "application/json")
            .body(redeeming(&code, "flight", "a long enough password"))
            .send()
            .await
            .expect("an answer over TLS");
        assert_eq!(redeemed.status(), reqwest::StatusCode::NO_CONTENT);

        let signed_in = client
            .post(format!("{at}/api/sign-in"))
            .header("content-type", "application/json")
            .body(signing_in("flight", "a long enough password"))
            .send()
            .await
            .expect("an answer over TLS");
        assert_eq!(signed_in.status(), reqwest::StatusCode::NO_CONTENT);

        let cookie = signed_in
            .headers()
            .get("set-cookie")
            .expect("a cookie")
            .to_str()
            .expect("a readable cookie")
            .to_owned();
        assert!(cookie.contains("Secure"), "{cookie}");

        let signed_out = client
            .post(format!("{at}/api/sign-out"))
            .header("cookie", cookie.split(';').next().expect("a value"))
            .send()
            .await
            .expect("an answer over TLS");
        assert_eq!(signed_out.status(), reqwest::StatusCode::NO_CONTENT);

        serving.stop().await;
    }
}
