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

mod administration;
mod answers;
mod assets;
mod bootstrap;
mod cookies;
mod enrolment;
mod liveness;
mod password;
mod principal;
mod rate_limit;
mod routes;
mod sign_in;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum_server::Handle;
use axum_server::tls_rustls::RustlsConfig;
use tokio::task::JoinHandle;

use crate::authorisation::Requirement;
use axum::response::Response;

use crate::configuration::{Deployment, Store, StoreError, Transaction, UserId, Users};
use crate::identity::{Bootstrap, Identity, PasswordRefused};
use crate::telemetry::module;
use rate_limit::{Admission, RateLimits};
use routes::RouteTable;

/// Everything a handler is given, and nothing it is not.
///
/// Transport may call Authorisation, Configuration, Identity, the state authority and
/// Synthesis ([`docs/spec/modules.md`]), and what it holds of each of them is here. A handler
/// reaches nothing that is not in this struct.
///
/// [`docs/spec/modules.md`]: ../../../docs/spec/modules.md
#[derive(Clone)]
struct Api {
    store: Arc<Store>,
    identity: Identity,
    limits: Arc<RateLimits>,
    /// The code this run of the process minted, where nobody administers this deployment
    /// yet. `None` is the ordinary state, and it is what leaves the redemption route
    /// unregistered rather than merely refusing.
    bootstrap: Option<Arc<Bootstrap>>,
}

impl Api {
    /// Whether this source may attempt something that presents a credential.
    ///
    /// Keyed on the source and never on the name submitted, so no number of failures can
    /// lock anybody out ([ADR-0025]).
    ///
    /// [ADR-0025]: ../../../docs/adr/0025-credentials-are-administered-because-there-is-no-email.md
    fn admits(&self, source: &SocketAddr) -> bool {
        self.limits.admit(source.ip()) == Admission::Permitted
    }
}

/// Why VoxLoop will not store a password, said to whoever offered one.
///
/// Two routes set a password — redeeming an enrolment code and changing one's own — and they
/// differ in what they have open to abandon, never in what makes a password unstorable. Only
/// the second half is shared, so each caller still rolls back its own transaction: a refusal
/// that left a spent enrolment code behind would cost somebody their only way in over a typo.
fn unstorable(refusal: &PasswordRefused) -> Response {
    match refusal {
        PasswordRefused::TooShort => answers::cannot(&refusal.to_string()),
        PasswordRefused::Unusable => {
            tracing::error!(target: module::IDENTITY, "a password could not be hashed");
            answers::cannot("That password could not be stored.")
        }
    }
}

/// The name to snapshot into an audit entry: the one the store holds, not the one submitted.
///
/// The log outlives the records it references ([ADR-0028]), so every entry carries the name
/// as it stood alongside the internal id that stays correct across a rename.
///
/// [ADR-0028]: ../../../docs/adr/0028-the-audit-log-records-decisions-not-traffic.md
async fn name_as_it_stands(
    transaction: &mut Transaction,
    user: &UserId,
) -> Result<String, StoreError> {
    Ok(transaction
        .user(user)
        .await?
        .map_or_else(String::new, |user| user.username))
}

/// Tell *leave it alone* apart from *take it away* in an edit.
///
/// serde cannot: an absent field and an explicit `null` both arrive as `None`, and an edit
/// that omits a field means leave it while one that sends `null` means unset it. The outer
/// `Option` is presence and the inner is the value, so the two are different answers rather
/// than the same one read twice.
fn present_or_absent<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    serde::Deserialize::deserialize(deserializer).map(Some)
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
    use Requirement::{Public, SignedIn, SystemAdministration};
    use administration::{grid, loops, roles, users};

    let mut table = RouteTable::new(Arc::clone(&api.store))
        .get("/api/liveness", Public, liveness::liveness)
        .post("/api/sign-in", Public, sign_in::sign_in)
        .post("/api/sign-out", SignedIn, sign_in::sign_out)
        // The credential lifetime of everybody who is not the first administrator: an
        // administrator issues a code, whoever holds it redeems it, and a signed-in user
        // changes their own by re-presenting it. There is no self-registration route and no
        // self-service reset route, because there is no mail path to carry one ([ADR-0025]).
        .post("/api/enrolment", Public, enrolment::redeem)
        .get("/api/principal", SignedIn, principal::own)
        .post("/api/password", SignedIn, password::change)
        // System administration. Every one of these is gated on the user's flag and never on
        // a role (v1 §9), so the console opens from the lobby and from within a session
        // alike. Every write is audited; the two reads are not.
        .get("/api/users", SystemAdministration, users::list)
        .post("/api/users", SystemAdministration, users::create_account)
        .get("/api/users/{id}", SystemAdministration, users::read)
        .patch("/api/users/{id}", SystemAdministration, users::edit)
        .delete("/api/users/{id}", SystemAdministration, users::delete)
        .post("/api/users/{id}/lock", SystemAdministration, users::lock)
        .post(
            "/api/users/{id}/unlock",
            SystemAdministration,
            users::unlock,
        )
        .post(
            "/api/users/{id}/force-password-reset",
            SystemAdministration,
            users::force_password_reset,
        )
        .post(
            "/api/users/{id}/enrolment-code",
            SystemAdministration,
            users::issue_enrolment_code,
        )
        // Roles and loops: the two configuration objects voice authority is expressed over.
        // Which role may hear or say what on which loop is the grid, and it is not here.
        .get("/api/roles", SystemAdministration, roles::list)
        .post("/api/roles", SystemAdministration, roles::create_role)
        .get("/api/roles/{id}", SystemAdministration, roles::read)
        .patch("/api/roles/{id}", SystemAdministration, roles::edit)
        .delete("/api/roles/{id}", SystemAdministration, roles::delete)
        // The base order is registered before `{id}`, because it is the one path under
        // `/api/loops/` that names something other than a loop.
        .put("/api/loops/order", SystemAdministration, loops::set_order)
        .get("/api/loops", SystemAdministration, loops::list)
        .post("/api/loops", SystemAdministration, loops::create_loop)
        .get("/api/loops/{id}", SystemAdministration, loops::read)
        .patch("/api/loops/{id}", SystemAdministration, loops::edit)
        .delete("/api/loops/{id}", SystemAdministration, loops::delete)
        // The grid: one value per (role, loop), and the only place voice authority is
        // configured. The console reads it a row or a column at a time ([ADR-0015]), so the
        // two reads administrators work from hang off the record whose page they are, and
        // the whole-grid read is its own route because reviewing is not administering.
        .get("/api/roles/{id}/grid", SystemAdministration, grid::row)
        .get("/api/loops/{id}/grid", SystemAdministration, grid::column)
        .get("/api/grid", SystemAdministration, grid::matrix)
        // A cell is addressed by its pair and holds exactly one value, so it is replaced
        // rather than patched. There is no route that clears one: setting `none` is how a
        // permission is taken away.
        .put("/api/grid/{role}/{loop}", SystemAdministration, grid::set)
        // Ruling on a loop's column, which writes cells rather than the loop.
        .post(
            "/api/loops/{id}/dismiss-unreviewed",
            SystemAdministration,
            grid::dismiss_unreviewed,
        );

    // Registered only while no system administrator exists, and genuinely absent otherwise:
    // the one operation VoxLoop hides rather than refuses (v1 §3).
    if api.bootstrap.is_some() {
        table = table.post("/api/bootstrap", Public, bootstrap::redeem);
    }

    table.fallback(Public, assets::bundle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    use std::net::IpAddr;

    use axum::body::Body;
    use axum::extract::ConnectInfo;
    use axum::http::{Request, StatusCode, header};
    use axum::response::Response;

    use crate::configuration::{AuditEvent, AuditLog, NewUser, Snapshot, Users, a_temporary_store};

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
                            .hash_password("a long enough password")
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

        /// Somebody who can sign in, which is a record with a password on it.
        ///
        /// Written straight to the store because the console creates users with no password
        /// and an enrolment code sets one (#32), so there is no route that does this yet.
        async fn a_user_who_can_sign_in(&self, username: &str, administers: bool) -> String {
            let hashed = self
                .api
                .identity
                .hash_password("a long enough password")
                .expect("the password to hash");
            let mut transaction = self.api.store.begin().await.expect("a transaction");
            let id = transaction
                .create_user(NewUser {
                    username: username.to_owned(),
                    password_hash: Some(hashed),
                    is_system_administrator: administers,
                })
                .await
                .expect("the user to be created");
            transaction.commit().await.expect("the user to land");

            id.as_str().to_owned()
        }

        /// Sign somebody in, and answer with the cookie a browser would present back.
        async fn signed_in_as(&self, username: &str) -> String {
            let signed_in = self
                .post(
                    "/api/sign-in",
                    &signing_in(username, "a long enough password"),
                )
                .await;
            assert_eq!(
                signed_in.status,
                StatusCode::NO_CONTENT,
                "{username} could not sign in"
            );

            signed_in.presented()
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
            self.holding(cookie, "POST", path, body).await
        }

        async fn get_holding(&self, cookie: &str, path: &str) -> Answer {
            self.holding(cookie, "GET", path, "").await
        }

        async fn holding(&self, cookie: &str, method: &str, path: &str, body: &str) -> Answer {
            self.ask(
                from("192.0.2.1")
                    .method(method)
                    .uri(path)
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::COOKIE, cookie)
                    .body(Body::from(body.to_owned()))
                    .unwrap(),
            )
            .await
        }

        /// Everything the audit log holds, newest first.
        async fn audited(&self) -> Vec<(AuditEvent, String, Option<IpAddr>)> {
            self.entries()
                .await
                .into_iter()
                .map(|entry| (entry.event, entry.actor_name, entry.source))
                .collect()
        }

        /// The entries themselves, for the promises that are about what an entry holds.
        async fn entries(&self) -> Vec<crate::configuration::RecordedEntry> {
            let mut transaction = self.api.store.begin().await.expect("a transaction");
            let entries = transaction
                .recent_entries(50)
                .await
                .expect("the log to be readable");
            transaction.roll_back().await.expect("the read to finish");

            entries
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

        assert_eq!(again.status, StatusCode::UNAUTHORIZED);
        assert_eq!(again.body, "That is not this server's bootstrap code.\n");
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
        assert_eq!(guessed.status, StatusCode::UNAUTHORIZED);

        let redeemed = box_of
            .post(
                "/api/bootstrap",
                &redeeming(&code, "flight", "a long enough password"),
            )
            .await;
        assert_eq!(redeemed.status, StatusCode::NO_CONTENT);
    }

    /// A wrong code learns nothing. Whether a username is taken is not something an
    /// unauthenticated caller is entitled to find out, which is the same rule the decoy hash
    /// keeps on the sign-in route.
    #[tokio::test]
    async fn a_wrong_code_answers_a_taken_name_exactly_as_it_answers_a_free_one() {
        let box_of = ABox::with_nobody_on_it().await;
        let mut transaction = box_of.api.store.begin().await.expect("a transaction");
        transaction
            .create_user(NewUser {
                username: "taken".to_owned(),
                password_hash: None,
                is_system_administrator: false,
            })
            .await
            .expect("an ordinary user");
        transaction.commit().await.expect("the user to land");

        let against_a_taken_name = box_of
            .post(
                "/api/bootstrap",
                &redeeming("not the code", "taken", "a long enough password"),
            )
            .await;
        let against_a_free_name = box_of
            .post(
                "/api/bootstrap",
                &redeeming("not the code", "free", "a long enough password"),
            )
            .await;

        assert_eq!(against_a_taken_name.status, against_a_free_name.status);
        assert_eq!(against_a_taken_name.body, against_a_free_name.body);

        // The code is what entitles the caller to the difference.
        let code = box_of.bootstrap_code();
        let told = box_of
            .post(
                "/api/bootstrap",
                &redeeming(&code, "taken", "a long enough password"),
            )
            .await;
        assert_eq!(told.status, StatusCode::BAD_REQUEST);
        assert!(told.body.contains("taken"), "{:?}", told.body);
    }

    #[tokio::test]
    async fn a_refused_code_is_audited() {
        let box_of = ABox::with_nobody_on_it().await;

        box_of
            .post_from(
                "198.51.100.9",
                "/api/bootstrap",
                &redeeming("not the code", "hopeful", "a long enough password"),
            )
            .await;

        assert_eq!(
            box_of.audited().await,
            [(
                AuditEvent::BootstrapRefused,
                "hopeful".to_owned(),
                Some(IpAddr::from([198, 51, 100, 9]))
            )]
        );
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
                Some(IpAddr::from([192, 0, 2, 1]))
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
        assert_eq!(answer.body, "That operation is for a signed-in user.\n");
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
                Some(IpAddr::from([192, 0, 2, 1]))
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
                Some(IpAddr::from([198, 51, 100, 9]))
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

    // ---- #31: the admin console shell and user administration -------------------------

    fn an_account(username: &str, administers: bool) -> String {
        format!(r#"{{"username":"{username}","system_administration":{administers}}}"#)
    }

    /// The id of the user the console just made, read out of what it answered.
    fn id_in(body: &str) -> String {
        let at = body.find("\"id\":\"").expect("an id in the answer") + 6;
        body[at..].split('"').next().expect("the id").to_owned()
    }

    /// The console is gated on the system-administration flag **alone** (v1 §9). This
    /// administrator has assumed no role, which is what the lobby is, and the console opens.
    #[tokio::test]
    async fn the_admin_console_opens_on_the_flag_alone_and_never_on_a_role() {
        let box_of = ABox::already_administered().await;
        let held = box_of.signed_in_as("root").await;

        let answer = box_of.get_holding(&held, "/api/users").await;

        assert_eq!(answer.status, StatusCode::OK);
        assert!(answer.body.contains("root"), "{:?}", answer.body);
    }

    #[tokio::test]
    async fn a_signed_in_user_without_the_flag_may_not_open_the_console() {
        let box_of = ABox::already_administered().await;
        box_of.a_user_who_can_sign_in("flight", false).await;
        let held = box_of.signed_in_as("flight").await;

        let answer = box_of.get_holding(&held, "/api/users").await;

        assert_eq!(answer.status, StatusCode::FORBIDDEN);
        assert!(
            answer.body.contains("system administrator"),
            "{:?}",
            answer.body
        );
    }

    /// What the console frame asks to know whether it exists for this person, and the flag
    /// comes from the store rather than from the cookie, which carries no claims (v1 §3).
    #[tokio::test]
    async fn the_signed_in_user_is_told_whether_they_hold_the_flag() {
        let box_of = ABox::already_administered().await;
        box_of.a_user_who_can_sign_in("flight", false).await;

        let administrator = box_of
            .get_holding(&box_of.signed_in_as("root").await, "/api/principal")
            .await;
        let operator = box_of
            .get_holding(&box_of.signed_in_as("flight").await, "/api/principal")
            .await;

        assert_eq!(administrator.status, StatusCode::OK);
        assert!(
            administrator
                .body
                .contains(r#""system_administration":true"#),
            "{:?}",
            administrator.body
        );
        assert!(
            operator.body.contains(r#""system_administration":false"#),
            "{:?}",
            operator.body
        );
    }

    #[tokio::test]
    async fn creates_reads_edits_and_deletes_a_user() {
        let box_of = ABox::already_administered().await;
        let held = box_of.signed_in_as("root").await;

        let created = box_of
            .post_holding(&held, "/api/users", &an_account("flight", false))
            .await;
        assert_eq!(created.status, StatusCode::CREATED);
        let id = id_in(&created.body);

        let read = box_of.get_holding(&held, &format!("/api/users/{id}")).await;
        assert_eq!(read.status, StatusCode::OK);
        assert!(read.body.contains("flight"), "{:?}", read.body);

        let edited = box_of
            .holding(
                &held,
                "PATCH",
                &format!("/api/users/{id}"),
                r#"{"username":"flight-director","system_administration":true}"#,
            )
            .await;
        assert_eq!(edited.status, StatusCode::OK);
        assert!(edited.body.contains("flight-director"), "{:?}", edited.body);

        let deleted = box_of
            .holding(&held, "DELETE", &format!("/api/users/{id}"), "")
            .await;
        assert_eq!(deleted.status, StatusCode::NO_CONTENT);
        assert_eq!(
            box_of
                .get_holding(&held, &format!("/api/users/{id}"))
                .await
                .status,
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn a_username_already_taken_is_refused_and_the_refusal_is_audited() {
        let box_of = ABox::already_administered().await;
        let held = box_of.signed_in_as("root").await;

        let answer = box_of
            .post_holding(&held, "/api/users", &an_account("root", false))
            .await;

        assert_eq!(answer.status, StatusCode::BAD_REQUEST);
        assert!(answer.body.contains("taken"), "{:?}", answer.body);
        let entries = box_of.entries().await;
        let write = entries[0].write.as_ref().expect("a configuration write");
        assert_eq!(entries[0].event, AuditEvent::UserCreated);
        assert!(write.refusal.is_some(), "the refusal was not recorded");
        assert_eq!(write.after, None, "a refused write recorded an after");
    }

    /// Locking ends the sign-in and the session immediately (v1 §2's lifetime table), so the
    /// cookie the locked user is holding stops working on their very next request.
    #[tokio::test]
    async fn locking_an_account_ends_the_sign_in_it_holds_and_the_next_one_attempted() {
        let box_of = ABox::already_administered().await;
        let id = box_of.a_user_who_can_sign_in("flight", false).await;
        let theirs = box_of.signed_in_as("flight").await;
        let held = box_of.signed_in_as("root").await;

        let locked = box_of
            .post_holding(&held, &format!("/api/users/{id}/lock"), "")
            .await;

        assert_eq!(locked.status, StatusCode::OK);
        assert_eq!(
            box_of
                .post_holding(&theirs, "/api/sign-out", "")
                .await
                .status,
            StatusCode::FORBIDDEN,
            "a locked account was still signed in"
        );
        assert_eq!(
            box_of
                .post(
                    "/api/sign-in",
                    &signing_in("flight", "a long enough password")
                )
                .await
                .status,
            StatusCode::UNAUTHORIZED,
            "a locked account could still sign in"
        );
    }

    #[tokio::test]
    async fn unlocking_an_account_lets_them_sign_in_again() {
        let box_of = ABox::already_administered().await;
        let id = box_of.a_user_who_can_sign_in("flight", false).await;
        let held = box_of.signed_in_as("root").await;
        box_of
            .post_holding(&held, &format!("/api/users/{id}/lock"), "")
            .await;

        let unlocked = box_of
            .post_holding(&held, &format!("/api/users/{id}/unlock"), "")
            .await;

        assert_eq!(unlocked.status, StatusCode::OK);
        assert_eq!(
            box_of
                .post(
                    "/api/sign-in",
                    &signing_in("flight", "a long enough password")
                )
                .await
                .status,
            StatusCode::NO_CONTENT
        );
    }

    #[tokio::test]
    async fn forcing_a_password_reset_ends_the_sign_in_and_takes_the_password_away() {
        let box_of = ABox::already_administered().await;
        let id = box_of.a_user_who_can_sign_in("flight", false).await;
        let theirs = box_of.signed_in_as("flight").await;
        let held = box_of.signed_in_as("root").await;

        let forced = box_of
            .post_holding(&held, &format!("/api/users/{id}/force-password-reset"), "")
            .await;

        assert_eq!(forced.status, StatusCode::OK);
        assert_eq!(
            box_of
                .post_holding(&theirs, "/api/sign-out", "")
                .await
                .status,
            StatusCode::FORBIDDEN,
            "the sign-in survived a forced reset"
        );
        assert_eq!(
            box_of
                .post(
                    "/api/sign-in",
                    &signing_in("flight", "a long enough password")
                )
                .await
                .status,
            StatusCode::UNAUTHORIZED,
            "the old password still signed them in"
        );
        assert!(
            box_of
                .entries()
                .await
                .iter()
                .any(|entry| entry.event == AuditEvent::PasswordResetForced),
            "the forced reset was not audited"
        );
    }

    /// The last system administrator cannot be removed (v1 §2), and each of the three acts
    /// that would remove them says *you may not* with the reason rather than hiding.
    #[tokio::test]
    async fn the_last_system_administrator_cannot_be_locked_deleted_or_demoted() {
        let box_of = ABox::already_administered().await;
        let held = box_of.signed_in_as("root").await;
        let id = id_in(&box_of.get_holding(&held, "/api/users").await.body);

        let locked = box_of
            .post_holding(&held, &format!("/api/users/{id}/lock"), "")
            .await;
        let demoted = box_of
            .holding(
                &held,
                "PATCH",
                &format!("/api/users/{id}"),
                r#"{"system_administration":false}"#,
            )
            .await;
        let deleted = box_of
            .holding(&held, "DELETE", &format!("/api/users/{id}"), "")
            .await;

        for refused in [&locked, &demoted, &deleted] {
            assert_eq!(refused.status, StatusCode::FORBIDDEN);
            assert!(
                refused.body.contains("last system administrator"),
                "{:?}",
                refused.body
            );
        }
        assert_eq!(
            box_of.get_holding(&held, "/api/users").await.status,
            StatusCode::OK,
            "the last administrator lost the console"
        );
    }

    /// An edit is one write: the rename and the flag land together or neither does, which is
    /// what makes the audit entry's before and after a true account of what happened.
    #[tokio::test]
    async fn an_edit_refused_halfway_leaves_the_record_exactly_as_it_was() {
        let box_of = ABox::already_administered().await;
        let held = box_of.signed_in_as("root").await;
        let id = id_in(&box_of.get_holding(&held, "/api/users").await.body);

        let refused = box_of
            .holding(
                &held,
                "PATCH",
                &format!("/api/users/{id}"),
                r#"{"username":"renamed","system_administration":false}"#,
            )
            .await;

        assert_eq!(refused.status, StatusCode::FORBIDDEN);
        let read = box_of.get_holding(&held, &format!("/api/users/{id}")).await;
        assert!(
            read.body.contains("root"),
            "the rename landed: {:?}",
            read.body
        );
        assert!(
            read.body.contains(r#""system_administration":true"#),
            "{:?}",
            read.body
        );
    }

    /// Every write is audited with before and after **plus a blast radius passed into the
    /// transaction as a value**, and the write and its entry commit together (v1 §12).
    #[tokio::test]
    async fn every_administration_write_is_audited_with_before_after_and_a_blast_radius() {
        let box_of = ABox::already_administered().await;
        let held = box_of.signed_in_as("root").await;
        let id = id_in(
            &box_of
                .post_holding(&held, "/api/users", &an_account("flight", false))
                .await
                .body,
        );

        box_of
            .holding(
                &held,
                "PATCH",
                &format!("/api/users/{id}"),
                r#"{"username":"flight-director"}"#,
            )
            .await;
        box_of
            .post_holding(&held, &format!("/api/users/{id}/lock"), "")
            .await;

        let entries = box_of.entries().await;
        let written: Vec<AuditEvent> = entries
            .iter()
            .filter(|entry| entry.write.is_some())
            .map(|entry| entry.event)
            .collect();
        assert_eq!(
            written,
            [
                AuditEvent::AccountLocked,
                AuditEvent::UserEdited,
                AuditEvent::UserCreated
            ]
        );
        for entry in entries.iter().filter(|entry| entry.write.is_some()) {
            let write = entry.write.as_ref().expect("a configuration write");
            assert!(!write.target_name.is_empty(), "an entry named no target");
            assert!(write.after.is_some(), "{:?} recorded no after", entry.event);
            assert_eq!(
                write.blast_radius,
                crate::configuration::BlastRadius::nothing_live(),
                "no session exists, so nothing live was touched"
            );
        }
        let edited = &entries[1].write.as_ref().expect("a configuration write");
        assert_ne!(edited.before, edited.after, "the edit recorded no change");
    }

    /// A refused read is not audited — a denied read is usually a stale browser tab (v1 §3).
    #[tokio::test]
    async fn a_refused_read_is_not_audited() {
        let box_of = ABox::already_administered().await;
        box_of.a_user_who_can_sign_in("flight", false).await;
        let held = box_of.signed_in_as("flight").await;
        let before = box_of.entries().await.len();

        box_of.get_holding(&held, "/api/users").await;

        assert_eq!(box_of.entries().await.len(), before);
    }

    /// A refused *write* is, even where the refusal came before the handler (v1 §3). An
    /// unauthorised attempt to make an administrator is the case worth keeping, and the
    /// entry names who tried, from where, and at what.
    #[tokio::test]
    async fn an_administration_write_refused_for_want_of_the_flag_is_audited() {
        let box_of = ABox::already_administered().await;
        box_of.a_user_who_can_sign_in("flight", false).await;
        let held = box_of.signed_in_as("flight").await;

        let refused = box_of
            .post_holding(&held, "/api/users", &an_account("a-second-root", true))
            .await;

        assert_eq!(refused.status, StatusCode::FORBIDDEN);
        let entries = box_of.entries().await;
        assert_eq!(entries[0].event, AuditEvent::AdministrationRefused);
        assert_eq!(entries[0].actor_name, "flight");
        assert_eq!(entries[0].operation.as_deref(), Some("POST /api/users"));
        assert!(
            entries[0].write.is_none(),
            "nothing was written, so nothing about a record should be"
        );
    }

    /// Nobody signed in at all is still recorded, by where they came from.
    #[tokio::test]
    async fn an_administration_write_attempted_by_nobody_is_audited_against_its_source() {
        let box_of = ABox::already_administered().await;

        box_of
            .post_from("198.51.100.9", "/api/users", &an_account("hopeful", true))
            .await;

        let entries = box_of.entries().await;
        assert_eq!(entries[0].event, AuditEvent::AdministrationRefused);
        assert_eq!(entries[0].actor, None);
        assert_eq!(entries[0].source, Some(IpAddr::from([198, 51, 100, 9])));
    }

    /// A forced password reset changes nothing else about the record, so the snapshot has to
    /// carry whether a password is set or the entry records two identical lines and v1 §12's
    /// "before and after" says nothing.
    #[tokio::test]
    async fn a_forced_password_reset_is_audited_as_a_change_rather_than_as_two_identical_lines() {
        let box_of = ABox::already_administered().await;
        let id = box_of.a_user_who_can_sign_in("flight", false).await;
        let held = box_of.signed_in_as("root").await;

        box_of
            .post_holding(&held, &format!("/api/users/{id}/force-password-reset"), "")
            .await;

        let entries = box_of.entries().await;
        let write = entries[0].write.as_ref().expect("a configuration write");
        assert_eq!(entries[0].event, AuditEvent::PasswordResetForced);
        assert_ne!(write.before, write.after, "the reset recorded no change");
    }

    /// The log outlives the records it references ([ADR-0028]): the entries about a deleted
    /// user stay, carrying the internal id and the name as it stood.
    ///
    /// [ADR-0028]: ../../../docs/adr/0028-the-audit-log-records-decisions-not-traffic.md
    #[tokio::test]
    async fn deleting_a_user_leaves_their_entries_readable_and_attributed() {
        let box_of = ABox::already_administered().await;
        let held = box_of.signed_in_as("root").await;
        let id = id_in(
            &box_of
                .post_holding(&held, "/api/users", &an_account("flight", false))
                .await
                .body,
        );

        box_of
            .holding(&held, "DELETE", &format!("/api/users/{id}"), "")
            .await;

        let entries = box_of.entries().await;
        let about_them: Vec<&crate::configuration::RecordedEntry> = entries
            .iter()
            .filter(|entry| {
                entry
                    .write
                    .as_ref()
                    .is_some_and(|write| write.target_name == "flight")
            })
            .collect();
        assert_eq!(about_them.len(), 2, "the entries went with the user");
        for entry in about_them {
            let write = entry.write.as_ref().expect("a configuration write");
            assert_eq!(
                write.target.as_ref().map(|id| id.as_str()),
                Some(id.as_str())
            );
        }
    }

    #[tokio::test]
    async fn an_edit_that_asks_for_no_change_is_refused_rather_than_audited() {
        let box_of = ABox::already_administered().await;
        let held = box_of.signed_in_as("root").await;
        let id = id_in(&box_of.get_holding(&held, "/api/users").await.body);
        let before = box_of.entries().await.len();

        let answer = box_of
            .holding(&held, "PATCH", &format!("/api/users/{id}"), "{}")
            .await;

        assert_eq!(answer.status, StatusCode::BAD_REQUEST);
        assert_eq!(box_of.entries().await.len(), before);
    }

    #[tokio::test]
    async fn administering_a_user_nobody_holds_says_so_rather_than_pretending_to_have_done_it() {
        let box_of = ABox::already_administered().await;
        let held = box_of.signed_in_as("root").await;

        for (method, path) in [
            ("GET", "/api/users/nobody"),
            ("DELETE", "/api/users/nobody"),
            ("POST", "/api/users/nobody/lock"),
            ("POST", "/api/users/nobody/force-password-reset"),
        ] {
            let answer = box_of.holding(&held, method, path, "").await;

            assert_eq!(
                answer.status,
                StatusCode::NOT_FOUND,
                "expected {method} {path} to say there is no such user"
            );
        }
    }

    // ---- #32: enrolment codes, own-password change, and the routes that do not exist ----

    /// The code the console just issued, read out of what it answered. It is the only time
    /// anything hands one back.
    fn code_in(body: &str) -> String {
        let at = body.find("\"code\":\"").expect("a code in the answer") + 8;
        body[at..].split('"').next().expect("the code").to_owned()
    }

    fn redeeming_with(code: &str, password: &str) -> String {
        format!(r#"{{"code":"{code}","password":"{password}"}}"#)
    }

    fn changing(current: &str, new: &str) -> String {
        format!(r#"{{"current":"{current}","new":"{new}"}}"#)
    }

    impl ABox {
        /// Create a user with no password, the only way the console makes one.
        async fn a_user_awaiting_enrolment(&self, held: &str, username: &str) -> String {
            let created = self
                .post_holding(held, "/api/users", &an_account(username, false))
                .await;
            assert_eq!(created.status, StatusCode::CREATED, "{:?}", created.body);

            id_in(&created.body)
        }

        /// Issue an enrolment code against a user, and answer with the code itself.
        async fn a_code_for(&self, held: &str, id: &str) -> String {
            let issued = self
                .post_holding(held, &format!("/api/users/{id}/enrolment-code"), "")
                .await;
            assert_eq!(issued.status, StatusCode::CREATED, "{:?}", issued.body);

            code_in(&issued.body)
        }
    }

    /// The whole of what #32 is for: a record system administration made, a code handed over
    /// out of band, and an account somebody can sign into at the end of it.
    #[tokio::test]
    async fn an_administrator_issues_a_code_and_redeeming_it_sets_the_password() {
        let box_of = ABox::already_administered().await;
        let held = box_of.signed_in_as("root").await;
        let flight = box_of.a_user_awaiting_enrolment(&held, "flight").await;
        let code = box_of.a_code_for(&held, &flight).await;

        let redeemed = box_of
            .post(
                "/api/enrolment",
                &redeeming_with(&code, "a long enough password"),
            )
            .await;

        assert_eq!(
            redeemed.status,
            StatusCode::NO_CONTENT,
            "{:?}",
            redeemed.body
        );
        let signed_in = box_of
            .post(
                "/api/sign-in",
                &signing_in("flight", "a long enough password"),
            )
            .await;
        assert_eq!(
            signed_in.status,
            StatusCode::NO_CONTENT,
            "{:?}",
            signed_in.body
        );
    }

    /// A reset is the same act again (v1 §2), which is the whole reason there is one route
    /// rather than an enrolment route and a reset route.
    #[tokio::test]
    async fn a_password_reset_is_the_same_act_again() {
        let box_of = ABox::already_administered().await;
        let held = box_of.signed_in_as("root").await;
        let flight = box_of.a_user_awaiting_enrolment(&held, "flight").await;
        let first = box_of.a_code_for(&held, &flight).await;
        box_of
            .post(
                "/api/enrolment",
                &redeeming_with(&first, "the first password"),
            )
            .await;
        box_of
            .post_holding(
                &held,
                &format!("/api/users/{flight}/force-password-reset"),
                "",
            )
            .await;

        let again = box_of.a_code_for(&held, &flight).await;
        let redeemed = box_of
            .post(
                "/api/enrolment",
                &redeeming_with(&again, "the second password"),
            )
            .await;

        assert_eq!(
            redeemed.status,
            StatusCode::NO_CONTENT,
            "{:?}",
            redeemed.body
        );
        assert_eq!(
            box_of
                .post("/api/sign-in", &signing_in("flight", "the first password"))
                .await
                .status,
            StatusCode::UNAUTHORIZED,
            "the password the reset took away still works"
        );
        assert_eq!(
            box_of
                .post("/api/sign-in", &signing_in("flight", "the second password"))
                .await
                .status,
            StatusCode::NO_CONTENT
        );
    }

    #[tokio::test]
    async fn an_enrolment_code_is_good_once_and_a_second_one_invalidates_the_first() {
        let box_of = ABox::already_administered().await;
        let held = box_of.signed_in_as("root").await;
        let flight = box_of.a_user_awaiting_enrolment(&held, "flight").await;
        let first = box_of.a_code_for(&held, &flight).await;
        let second = box_of.a_code_for(&held, &flight).await;

        assert_eq!(
            box_of
                .post(
                    "/api/enrolment",
                    &redeeming_with(&first, "a long enough password")
                )
                .await
                .status,
            StatusCode::UNAUTHORIZED,
            "the code a reissue replaced still works"
        );
        assert_eq!(
            box_of
                .post(
                    "/api/enrolment",
                    &redeeming_with(&second, "a long enough password")
                )
                .await
                .status,
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            box_of
                .post(
                    "/api/enrolment",
                    &redeeming_with(&second, "another long password")
                )
                .await
                .status,
            StatusCode::UNAUTHORIZED,
            "a code was spent twice"
        );
    }

    /// The code survives a password VoxLoop would not store. Spending it on a typo would cost
    /// somebody their only way in, and the administrator who issued it may not be in today.
    #[tokio::test]
    async fn a_password_under_the_floor_is_refused_without_spending_the_code() {
        let box_of = ABox::already_administered().await;
        let held = box_of.signed_in_as("root").await;
        let flight = box_of.a_user_awaiting_enrolment(&held, "flight").await;
        let code = box_of.a_code_for(&held, &flight).await;

        let refused = box_of
            .post("/api/enrolment", &redeeming_with(&code, "too short"))
            .await;

        assert_eq!(
            refused.status,
            StatusCode::BAD_REQUEST,
            "{:?}",
            refused.body
        );
        assert_eq!(
            box_of
                .post(
                    "/api/enrolment",
                    &redeeming_with(&code, "a long enough password")
                )
                .await
                .status,
            StatusCode::NO_CONTENT,
            "a refused password spent the code"
        );
    }

    #[tokio::test]
    async fn a_code_nobody_issued_is_refused_and_the_refusal_is_audited_against_its_source() {
        let box_of = ABox::already_administered().await;

        let refused = box_of
            .post_from(
                "198.51.100.7",
                "/api/enrolment",
                &redeeming_with("guessed", "a long enough password"),
            )
            .await;

        assert_eq!(refused.status, StatusCode::UNAUTHORIZED);
        assert!(
            refused.body.starts_with("That enrolment code is not one"),
            "{:?}",
            refused.body
        );
        let audited = box_of.audited().await;
        assert_eq!(
            audited.first(),
            Some(&(
                AuditEvent::EnrolmentRefused,
                String::new(),
                Some("198.51.100.7".parse::<IpAddr>().expect("an address"))
            )),
            "{audited:?}"
        );
    }

    /// The credential this account had is not the one it has now, so nothing standing against
    /// the old one is left standing.
    #[tokio::test]
    async fn redeeming_a_code_ends_the_sign_ins_standing_against_the_password_it_replaces() {
        let box_of = ABox::already_administered().await;
        let held = box_of.signed_in_as("root").await;
        let id = box_of.a_user_who_can_sign_in("flight", false).await;
        let stale = box_of.signed_in_as("flight").await;
        let code = box_of.a_code_for(&held, &id).await;

        box_of
            .post(
                "/api/enrolment",
                &redeeming_with(&code, "a brand new password"),
            )
            .await;

        assert_eq!(
            box_of.get_holding(&stale, "/api/principal").await.status,
            StatusCode::FORBIDDEN,
            "a sign-in survived the credential it stood against"
        );
    }

    #[tokio::test]
    async fn nobody_but_a_system_administrator_may_issue_an_enrolment_code() {
        let box_of = ABox::already_administered().await;
        let flight = box_of.a_user_who_can_sign_in("flight", false).await;
        let theirs = box_of.signed_in_as("flight").await;

        let by_nobody = box_of
            .post(&format!("/api/users/{flight}/enrolment-code"), "")
            .await;
        let by_an_operator = box_of
            .post_holding(&theirs, &format!("/api/users/{flight}/enrolment-code"), "")
            .await;

        assert_eq!(by_nobody.status, StatusCode::FORBIDDEN);
        assert_eq!(by_an_operator.status, StatusCode::FORBIDDEN);
        // Refused administration writes are audited, and issuing a credential is one.
        let refusals = box_of
            .audited()
            .await
            .into_iter()
            .filter(|(event, ..)| *event == AuditEvent::AdministrationRefused)
            .count();
        assert_eq!(refusals, 2, "a refused issue was not audited");
    }

    /// Issuing is an administration write, so it is audited with what it changed — and what
    /// it changed is which credential enrols this user, never the credential itself.
    #[tokio::test]
    async fn issuing_a_code_is_audited_with_what_it_replaced_and_never_with_the_code() {
        let box_of = ABox::already_administered().await;
        let held = box_of.signed_in_as("root").await;
        let flight = box_of.a_user_awaiting_enrolment(&held, "flight").await;
        let first = box_of.a_code_for(&held, &flight).await;
        let second = box_of.a_code_for(&held, &flight).await;

        let entries = box_of.entries().await;
        let issues: Vec<_> = entries
            .iter()
            .filter(|entry| entry.event == AuditEvent::EnrolmentCodeIssued)
            .collect();

        assert_eq!(issues.len(), 2);
        let reissue = issues.first().expect("the second issue");
        let write = reissue.write.as_ref().expect("a configuration write");
        assert_eq!(write.target_name, "flight");
        assert_eq!(reissue.actor_name, "root");
        assert!(
            write.before.is_some(),
            "the code it replaced was not recorded"
        );
        assert!(write.after.is_some());
        for entry in &entries {
            let held = format!("{entry:?}");
            assert!(!held.contains(&first), "the log holds a code");
            assert!(!held.contains(&second), "the log holds a code");
        }
    }

    /// The console is told a code is out there so an administrator does not issue a second
    /// and leave the first in somebody's hand. It is never told what the code is.
    #[tokio::test]
    async fn the_console_is_told_a_code_is_outstanding_and_never_what_it_is() {
        let box_of = ABox::already_administered().await;
        let held = box_of.signed_in_as("root").await;
        let flight = box_of.a_user_awaiting_enrolment(&held, "flight").await;

        let before = box_of
            .get_holding(&held, &format!("/api/users/{flight}"))
            .await;
        let code = box_of.a_code_for(&held, &flight).await;
        let after = box_of
            .get_holding(&held, &format!("/api/users/{flight}"))
            .await;
        let listed = box_of.get_holding(&held, "/api/users").await;

        assert!(
            before.body.contains(r#""enrolment_expires_at":null"#),
            "{:?}",
            before.body
        );
        assert!(
            !after.body.contains(r#""enrolment_expires_at":null"#),
            "{:?}",
            after.body
        );
        assert!(
            !after.body.contains(&code),
            "the console was handed the code back"
        );
        assert!(
            !listed.body.contains(&code),
            "the console was handed the code back"
        );
    }

    /// Displayed state is factual (v1's standing requirements). A write's answer says what
    /// the record and its code are, together — not what the record is and *nothing* about
    /// the code, which reads identically to *no code* and is not the same claim.
    #[tokio::test]
    async fn a_write_answers_with_the_code_outstanding_rather_than_with_silence() {
        let box_of = ABox::already_administered().await;
        let held = box_of.signed_in_as("root").await;
        let flight = box_of.a_user_awaiting_enrolment(&held, "flight").await;
        box_of.a_code_for(&held, &flight).await;

        let locked = box_of
            .post_holding(&held, &format!("/api/users/{flight}/lock"), "")
            .await;
        let renamed = box_of
            .holding(
                &held,
                "PATCH",
                &format!("/api/users/{flight}"),
                r#"{"username":"flight-director"}"#,
            )
            .await;

        for answer in [&locked, &renamed] {
            assert_eq!(answer.status, StatusCode::OK, "{:?}", answer.body);
            assert!(
                !answer.body.contains(r#""enrolment_expires_at":null"#),
                "a write said there was no code outstanding: {:?}",
                answer.body
            );
        }
    }

    // ---- Changing one's own password ---------------------------------------------------

    /// An operator on the air who changes their password should not lose audio for it, so the
    /// session survives — which is the whole difference between this and every other act on
    /// a password (v1 §2).
    #[tokio::test]
    async fn a_signed_in_user_changes_their_own_password_and_the_sign_in_survives() {
        let box_of = ABox::already_administered().await;
        box_of.a_user_who_can_sign_in("flight", false).await;
        let held = box_of.signed_in_as("flight").await;

        let changed = box_of
            .post_holding(
                &held,
                "/api/password",
                &changing("a long enough password", "a brand new password"),
            )
            .await;

        assert_eq!(changed.status, StatusCode::NO_CONTENT, "{:?}", changed.body);
        assert_eq!(
            box_of.get_holding(&held, "/api/principal").await.status,
            StatusCode::OK,
            "changing a password ended the session"
        );
        assert_eq!(
            box_of
                .post(
                    "/api/sign-in",
                    &signing_in("flight", "a brand new password")
                )
                .await
                .status,
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            box_of
                .post(
                    "/api/sign-in",
                    &signing_in("flight", "a long enough password")
                )
                .await
                .status,
            StatusCode::UNAUTHORIZED,
            "the password that was changed still works"
        );
    }

    #[tokio::test]
    async fn changing_a_password_needs_the_current_one_and_a_wrong_one_is_audited() {
        let box_of = ABox::already_administered().await;
        box_of.a_user_who_can_sign_in("flight", false).await;
        let held = box_of.signed_in_as("flight").await;

        let refused = box_of
            .post_holding(
                &held,
                "/api/password",
                &changing("not their password", "a brand new password"),
            )
            .await;

        assert_eq!(
            refused.status,
            StatusCode::UNAUTHORIZED,
            "{:?}",
            refused.body
        );
        assert_eq!(
            box_of
                .post(
                    "/api/sign-in",
                    &signing_in("flight", "a long enough password")
                )
                .await
                .status,
            StatusCode::NO_CONTENT,
            "a refused change wrote the new password anyway"
        );
        let audited = box_of.audited().await;
        assert!(
            audited.iter().any(
                |(event, name, source)| *event == AuditEvent::PasswordChangeRefused
                    && name == "flight"
                    && source.is_some()
            ),
            "{audited:?}"
        );
    }

    #[tokio::test]
    async fn a_new_password_under_the_floor_is_refused_and_leaves_the_old_one_standing() {
        let box_of = ABox::already_administered().await;
        box_of.a_user_who_can_sign_in("flight", false).await;
        let held = box_of.signed_in_as("flight").await;

        let refused = box_of
            .post_holding(
                &held,
                "/api/password",
                &changing("a long enough password", "too short"),
            )
            .await;

        assert_eq!(
            refused.status,
            StatusCode::BAD_REQUEST,
            "{:?}",
            refused.body
        );
        assert_eq!(
            box_of
                .post(
                    "/api/sign-in",
                    &signing_in("flight", "a long enough password")
                )
                .await
                .status,
            StatusCode::NO_CONTENT
        );
    }

    #[tokio::test]
    async fn nobody_who_is_not_signed_in_may_change_a_password() {
        let box_of = ABox::already_administered().await;
        box_of.a_user_who_can_sign_in("flight", false).await;

        let refused = box_of
            .post(
                "/api/password",
                &changing("a long enough password", "a brand new password"),
            )
            .await;

        assert_eq!(refused.status, StatusCode::FORBIDDEN);
        assert!(refused.body.contains("signed-in"), "{:?}", refused.body);
    }

    /// Both routes accept a credential, so both are throttled on source — and neither is
    /// throttled on the account, because no number of attempts locks anybody out (ADR-0025).
    #[tokio::test]
    async fn redemption_and_a_password_change_are_throttled_on_source() {
        let box_of = ABox::already_administered().await;
        box_of.a_user_who_can_sign_in("flight", false).await;
        let held = box_of.signed_in_as("flight").await;

        let mut throttled = None;
        for _ in 0..40 {
            let answer = box_of
                .post_from(
                    "198.51.100.9",
                    "/api/enrolment",
                    &redeeming_with("guessed", "a long enough password"),
                )
                .await;
            if answer.status == StatusCode::TOO_MANY_REQUESTS {
                throttled = Some(answer);
                break;
            }
        }
        assert!(throttled.is_some(), "redemption was never throttled");

        let mut change_throttled = false;
        for _ in 0..80 {
            let answer = box_of
                .post_holding(
                    &held,
                    "/api/password",
                    &changing("not their password", "a brand new password"),
                )
                .await;
            if answer.status == StatusCode::TOO_MANY_REQUESTS {
                change_throttled = true;
                break;
            }
        }
        assert!(change_throttled, "a password change was never throttled");

        // Throttled, never locked: the account is exactly as it was.
        assert_eq!(
            box_of
                .post_from(
                    "203.0.113.4",
                    "/api/sign-in",
                    &signing_in("flight", "a long enough password")
                )
                .await
                .status,
            StatusCode::NO_CONTENT
        );
    }

    // ---- What deliberately does not exist ----------------------------------------------

    /// There is no self-registration and no self-service reset, because there is no mail path
    /// to carry one (ADR-0025). Every plausible name for one answers as what it is: an
    /// operation VoxLoop does not have.
    #[tokio::test]
    async fn there_is_no_self_registration_route_and_no_self_service_reset_route() {
        let box_of = ABox::already_administered().await;

        for path in [
            "/api/register",
            "/api/sign-up",
            "/api/forgot-password",
            "/api/reset-password",
            "/api/password-reset",
        ] {
            let answer = box_of.post(path, r#"{"username":"flight"}"#).await;

            assert_eq!(
                answer.status,
                StatusCode::NOT_FOUND,
                "expected {path} not to exist, got {:?}",
                answer.body
            );
        }

        // The one route that creates a user is system administration, and it says so.
        assert_eq!(
            box_of
                .post("/api/users", &an_account("flight", false))
                .await
                .status,
            StatusCode::FORBIDDEN
        );
    }

    /// The code identifies the user, so a redemption has nobody to name — and nothing a
    /// caller adds to it aims the redemption anywhere else.
    #[tokio::test]
    async fn a_redemption_names_no_user_so_there_is_nobody_to_aim_it_at() {
        let box_of = ABox::already_administered().await;
        let held = box_of.signed_in_as("root").await;
        let flight = box_of.a_user_awaiting_enrolment(&held, "flight").await;
        let code = box_of.a_code_for(&held, &flight).await;

        let redeemed = box_of
            .post(
                "/api/enrolment",
                &format!(
                    r#"{{"code":"{code}","password":"a long enough password","username":"root"}}"#
                ),
            )
            .await;

        assert_eq!(
            redeemed.status,
            StatusCode::NO_CONTENT,
            "{:?}",
            redeemed.body
        );
        assert_eq!(
            box_of
                .post(
                    "/api/sign-in",
                    &signing_in("flight", "a long enough password")
                )
                .await
                .status,
            StatusCode::NO_CONTENT,
            "the code did not enrol the user it was issued against"
        );
        assert_eq!(
            box_of
                .post(
                    "/api/sign-in",
                    &signing_in("root", "a long enough password")
                )
                .await
                .status,
            StatusCode::NO_CONTENT,
            "the named user's password was touched"
        );
    }

    /// Every name in a list the console answered with, in the order it answered them.
    fn names_in(body: &str) -> Vec<String> {
        body.split("\"name\":\"")
            .skip(1)
            .map(|rest| rest.split('"').next().expect("the name").to_owned())
            .collect()
    }

    impl ABox {
        /// Make a loop through the console, and answer with its id.
        async fn a_loop_called(&self, held: &str, name: &str) -> String {
            let made = self
                .post_holding(held, "/api/loops", &format!(r#"{{"name":"{name}"}}"#))
                .await;
            assert_eq!(made.status, StatusCode::CREATED, "{:?}", made.body);

            id_in(&made.body)
        }
    }

    /// Install seeds `Observer` and nothing else — no loops, and so no reach for it to have
    /// been seeded against (v1 §9, ADR-0015).
    #[tokio::test]
    async fn install_seeds_the_observer_role_and_leaves_the_deployment_with_no_loops() {
        let box_of = ABox::already_administered().await;
        let held = box_of.signed_in_as("root").await;

        let roles = box_of.get_holding(&held, "/api/roles").await;
        let loops = box_of.get_holding(&held, "/api/loops").await;

        assert_eq!(names_in(&roles.body), ["Observer"]);
        assert!(
            roles.body.contains(r#""max_occupants":null"#),
            "the seeded Observer role carries a limit on how many may observe: {:?}",
            roles.body
        );
        assert_eq!(loops.body, "[]", "install seeded a loop");
    }

    #[tokio::test]
    async fn creates_reads_edits_and_deletes_a_role() {
        let box_of = ABox::already_administered().await;
        let held = box_of.signed_in_as("root").await;

        let created = box_of
            .post_holding(
                &held,
                "/api/roles",
                r#"{"name":"Flight Director","max_occupants":1}"#,
            )
            .await;
        assert_eq!(created.status, StatusCode::CREATED, "{:?}", created.body);
        let id = id_in(&created.body);

        let read = box_of.get_holding(&held, &format!("/api/roles/{id}")).await;
        assert_eq!(read.status, StatusCode::OK);
        assert!(read.body.contains("Flight Director"), "{:?}", read.body);

        // A rename and a change of limit are one act, and the limit is taken away rather
        // than left alone: `null` and an absent field are deliberately different answers.
        let edited = box_of
            .holding(
                &held,
                "PATCH",
                &format!("/api/roles/{id}"),
                r#"{"name":"Flight","max_occupants":null}"#,
            )
            .await;
        assert_eq!(edited.status, StatusCode::OK, "{:?}", edited.body);
        assert!(
            edited.body.contains(r#""name":"Flight""#),
            "{:?}",
            edited.body
        );
        assert!(
            edited.body.contains(r#""max_occupants":null"#),
            "{:?}",
            edited.body
        );

        let deleted = box_of
            .holding(&held, "DELETE", &format!("/api/roles/{id}"), "")
            .await;
        assert_eq!(deleted.status, StatusCode::NO_CONTENT);
        assert_eq!(
            box_of
                .get_holding(&held, &format!("/api/roles/{id}"))
                .await
                .status,
            StatusCode::NOT_FOUND
        );
    }

    /// An edit that leaves the limit alone is not one that takes it away.
    #[tokio::test]
    async fn renaming_a_role_without_naming_the_limit_leaves_the_limit_alone() {
        let box_of = ABox::already_administered().await;
        let held = box_of.signed_in_as("root").await;
        let id = id_in(
            &box_of
                .post_holding(
                    &held,
                    "/api/roles",
                    r#"{"name":"Support Engineer","max_occupants":6}"#,
                )
                .await
                .body,
        );

        let edited = box_of
            .holding(
                &held,
                "PATCH",
                &format!("/api/roles/{id}"),
                r#"{"name":"Support"}"#,
            )
            .await;

        assert!(
            edited.body.contains(r#""max_occupants":6"#),
            "{:?}",
            edited.body
        );
    }

    /// A role is a staffable position, so one nobody may occupy is not a role (v1 §1).
    #[tokio::test]
    async fn a_role_nobody_may_occupy_is_refused_and_the_refusal_is_audited() {
        let box_of = ABox::already_administered().await;
        let held = box_of.signed_in_as("root").await;

        let answer = box_of
            .post_holding(
                &held,
                "/api/roles",
                r#"{"name":"Nobody","max_occupants":0}"#,
            )
            .await;

        assert_eq!(answer.status, StatusCode::BAD_REQUEST);
        assert!(answer.body.contains("occupant"), "{:?}", answer.body);
        let entries = box_of.entries().await;
        assert_eq!(entries[0].event, AuditEvent::RoleCreated);
        let write = entries[0].write.as_ref().expect("a configuration write");
        assert!(write.refusal.is_some(), "the refusal was not recorded");
        assert_eq!(write.after, None, "a refused write recorded an after");
    }

    /// A loop created after install is `unreviewed` until an administrator has ruled on its
    /// column (v1 §9). Every loop is created after install, because install creates none.
    #[tokio::test]
    async fn creates_reads_edits_and_deletes_a_loop_which_arrives_unreviewed() {
        let box_of = ABox::already_administered().await;
        let held = box_of.signed_in_as("root").await;

        let created = box_of
            .post_holding(&held, "/api/loops", r#"{"name":"FLIGHT"}"#)
            .await;
        assert_eq!(created.status, StatusCode::CREATED, "{:?}", created.body);
        assert!(
            created.body.contains(r#""unreviewed":true"#),
            "a loop created after install was not unreviewed: {:?}",
            created.body
        );
        let id = id_in(&created.body);

        let read = box_of.get_holding(&held, &format!("/api/loops/{id}")).await;
        assert_eq!(read.status, StatusCode::OK);
        assert!(read.body.contains("FLIGHT"), "{:?}", read.body);

        let edited = box_of
            .holding(
                &held,
                "PATCH",
                &format!("/api/loops/{id}"),
                r#"{"name":"FLIGHT DIRECTOR"}"#,
            )
            .await;
        assert_eq!(edited.status, StatusCode::OK, "{:?}", edited.body);
        assert!(edited.body.contains("FLIGHT DIRECTOR"), "{:?}", edited.body);

        let deleted = box_of
            .holding(&held, "DELETE", &format!("/api/loops/{id}"), "")
            .await;
        assert_eq!(deleted.status, StatusCode::NO_CONTENT);
        assert_eq!(
            box_of
                .get_holding(&held, &format!("/api/loops/{id}"))
                .await
                .status,
            StatusCode::NOT_FOUND
        );
    }

    /// The base order is **administered, not derived** ([ADR-0053]): not alphabetical, and
    /// not creation order. A loop created afterwards lands at the end of it.
    ///
    /// [ADR-0053]: ../../../docs/adr/0053-the-loop-order-is-complete-and-a-new-loop-lands-at-the-end.md
    #[tokio::test]
    async fn the_base_loop_order_is_administered_and_a_new_loop_lands_at_the_end() {
        let box_of = ABox::already_administered().await;
        let held = box_of.signed_in_as("root").await;
        let gnc = box_of.a_loop_called(&held, "GNC").await;
        let flight = box_of.a_loop_called(&held, "FLIGHT").await;
        let thermal = box_of.a_loop_called(&held, "THERMAL").await;

        let set = box_of
            .holding(
                &held,
                "PUT",
                "/api/loops/order",
                &format!(r#"{{"order":["{thermal}","{gnc}","{flight}"]}}"#),
            )
            .await;
        assert_eq!(set.status, StatusCode::OK, "{:?}", set.body);

        box_of.a_loop_called(&held, "AIR").await;

        let read = box_of.get_holding(&held, "/api/loops").await;
        assert_eq!(
            names_in(&read.body),
            ["THERMAL", "GNC", "FLIGHT", "AIR"],
            "the loops came back in an order nobody administered"
        );
    }

    #[tokio::test]
    async fn an_order_that_does_not_name_every_loop_is_refused_and_audited() {
        let box_of = ABox::already_administered().await;
        let held = box_of.signed_in_as("root").await;
        let gnc = box_of.a_loop_called(&held, "GNC").await;
        box_of.a_loop_called(&held, "FLIGHT").await;

        let answer = box_of
            .holding(
                &held,
                "PUT",
                "/api/loops/order",
                &format!(r#"{{"order":["{gnc}"]}}"#),
            )
            .await;

        assert_eq!(answer.status, StatusCode::BAD_REQUEST);
        let entries = box_of.entries().await;
        assert_eq!(entries[0].event, AuditEvent::LoopOrderEdited);
        let write = entries[0].write.as_ref().expect("a configuration write");
        assert!(write.refusal.is_some(), "the refusal was not recorded");
        assert_eq!(write.after, None, "a refused order recorded an after");
        assert_eq!(
            names_in(&box_of.get_holding(&held, "/api/loops").await.body),
            ["GNC", "FLIGHT"],
            "a refused order was applied anyway"
        );
    }

    /// Every write is audited with before and after, roles and loops included (v1 §12). The
    /// order is the one write about no single record, so it names none and says what the
    /// order was either side instead.
    #[tokio::test]
    async fn every_role_and_loop_write_is_audited_with_before_and_after() {
        let box_of = ABox::already_administered().await;
        let held = box_of.signed_in_as("root").await;
        let role = id_in(
            &box_of
                .post_holding(&held, "/api/roles", r#"{"name":"Flight Director"}"#)
                .await
                .body,
        );
        box_of
            .holding(
                &held,
                "PATCH",
                &format!("/api/roles/{role}"),
                r#"{"max_occupants":2}"#,
            )
            .await;
        let first = box_of.a_loop_called(&held, "GNC").await;
        let second = box_of.a_loop_called(&held, "FLIGHT").await;
        box_of
            .holding(
                &held,
                "PUT",
                "/api/loops/order",
                &format!(r#"{{"order":["{second}","{first}"]}}"#),
            )
            .await;

        let entries = box_of.entries().await;
        let written: Vec<AuditEvent> = entries
            .iter()
            .filter(|entry| entry.write.is_some())
            .map(|entry| entry.event)
            .collect();
        assert_eq!(
            written,
            [
                AuditEvent::LoopOrderEdited,
                AuditEvent::LoopCreated,
                AuditEvent::LoopCreated,
                AuditEvent::RoleEdited,
                AuditEvent::RoleCreated,
            ]
        );
        for entry in entries.iter().filter(|entry| entry.write.is_some()) {
            let write = entry.write.as_ref().expect("a configuration write");
            assert!(!write.target_name.is_empty(), "an entry named no target");
            assert!(write.after.is_some(), "{:?} recorded no after", entry.event);
            assert_eq!(
                write.blast_radius,
                crate::configuration::BlastRadius::nothing_live(),
                "no session exists, so nothing live was touched"
            );
        }
        let ordered = entries[0].write.as_ref().expect("a configuration write");
        assert_eq!(ordered.target, None, "the order named a single loop");
        assert_eq!(
            ordered.before.as_ref().map(|order| order.as_str()),
            Some("loop_order=GNC, FLIGHT")
        );
        assert_eq!(
            ordered.after.as_ref().map(|order| order.as_str()),
            Some("loop_order=FLIGHT, GNC")
        );
        let edited = entries[3].write.as_ref().expect("a configuration write");
        assert_ne!(edited.before, edited.after, "the edit recorded no change");
    }

    /// Nothing joins on a name (v1 §1). A stray join on a username or a role name works
    /// perfectly until the first rename, so both are renamed here and everything that holds
    /// an id is read back through it.
    #[tokio::test]
    async fn a_username_or_role_rename_breaks_no_reference() {
        let box_of = ABox::already_administered().await;
        let held = box_of.signed_in_as("root").await;
        let user = id_in(
            &box_of
                .post_holding(&held, "/api/users", &an_account("flight", false))
                .await
                .body,
        );
        let role = id_in(
            &box_of
                .post_holding(&held, "/api/roles", r#"{"name":"Flight Director"}"#)
                .await
                .body,
        );
        let held_loop = box_of.a_loop_called(&held, "FLIGHT").await;

        box_of
            .holding(
                &held,
                "PATCH",
                &format!("/api/users/{user}"),
                r#"{"username":"flight-director"}"#,
            )
            .await;
        box_of
            .holding(
                &held,
                "PATCH",
                &format!("/api/roles/{role}"),
                r#"{"name":"Flight"}"#,
            )
            .await;
        box_of
            .holding(
                &held,
                "PATCH",
                &format!("/api/loops/{held_loop}"),
                r#"{"name":"FLIGHT DIRECTOR"}"#,
            )
            .await;

        // Every id still names what it named, and the console reads the new names through it.
        assert!(
            box_of
                .get_holding(&held, &format!("/api/users/{user}"))
                .await
                .body
                .contains("flight-director")
        );
        assert!(
            box_of
                .get_holding(&held, &format!("/api/roles/{role}"))
                .await
                .body
                .contains(r#""name":"Flight""#)
        );
        assert!(
            box_of
                .get_holding(&held, &format!("/api/loops/{held_loop}"))
                .await
                .body
                .contains("FLIGHT DIRECTOR")
        );
        // The order is held by id as well, so it survived all three renames.
        assert_eq!(
            names_in(&box_of.get_holding(&held, "/api/loops").await.body),
            ["FLIGHT DIRECTOR"]
        );

        // The audit entries about them still hold the ids, so the log did not follow a name.
        let entries = box_of.entries().await;
        let targets: Vec<&str> = entries
            .iter()
            .filter_map(|entry| entry.write.as_ref())
            .filter_map(|write| write.target.as_ref().map(|id| id.as_str()))
            .collect();
        for id in [&user, &role, &held_loop] {
            assert_eq!(
                targets.iter().filter(|target| *target == id).count(),
                2,
                "an entry about {id} lost the id it was written with: {targets:?}"
            );
        }
    }

    /// Roles and loops are `SystemAdministration` like every other configuration write, and
    /// a refused write is audited whatever it was about (v1 §3).
    #[tokio::test]
    async fn nobody_but_a_system_administrator_may_administer_roles_or_loops() {
        let box_of = ABox::already_administered().await;
        box_of.a_user_who_can_sign_in("flight", false).await;
        let theirs = box_of.signed_in_as("flight").await;

        for (path, body) in [
            ("/api/roles", r#"{"name":"Flight Director"}"#),
            ("/api/loops", r#"{"name":"FLIGHT"}"#),
        ] {
            let answer = box_of.post_holding(&theirs, path, body).await;

            assert_eq!(answer.status, StatusCode::FORBIDDEN, "{:?}", answer.body);
        }
        assert_eq!(
            box_of.get_holding(&theirs, "/api/roles").await.status,
            StatusCode::FORBIDDEN
        );

        let refused = box_of
            .entries()
            .await
            .into_iter()
            .filter(|entry| entry.event == AuditEvent::AdministrationRefused)
            .count();
        assert_eq!(
            refused, 2,
            "a refused write was not audited, or a refused read was"
        );
    }

    /// Every permission in a body the console answered with, in the order it answered them.
    fn permissions_in(body: &str) -> Vec<String> {
        body.split("\"permission\":\"")
            .skip(1)
            .map(|rest| rest.split('"').next().expect("the permission").to_owned())
            .collect()
    }

    impl ABox {
        /// Make a role through the console, and answer with its id.
        async fn a_role_called(&self, held: &str, name: &str) -> String {
            let made = self
                .post_holding(held, "/api/roles", &format!(r#"{{"name":"{name}"}}"#))
                .await;
            assert_eq!(made.status, StatusCode::CREATED, "{:?}", made.body);

            id_in(&made.body)
        }

        /// Make a loop and rule on its column.
        ///
        /// A loop nobody has ruled on is enforced as `none` on every rung whatever its cells
        /// say, so a test about a cell that skipped this would pass for the wrong reason.
        async fn a_ruled_on_loop_called(&self, held: &str, name: &str) -> String {
            let id = self.a_loop_called(held, name).await;
            let ruled = self
                .post_holding(held, &format!("/api/loops/{id}/dismiss-unreviewed"), "")
                .await;
            assert_eq!(ruled.status, StatusCode::OK, "{:?}", ruled.body);

            id
        }

        /// Set one cell, the way a role page or a loop page does.
        async fn sets(&self, held: &str, role: &str, on: &str, permission: &str) -> Answer {
            self.holding(
                held,
                "PUT",
                &format!("/api/grid/{role}/{on}"),
                &format!(r#"{{"permission":"{permission}"}}"#),
            )
            .await
        }
    }

    /// A role page **is** the row and a loop page **is** the column ([ADR-0015]) — the same
    /// cells read two ways, each as a list at full size rather than as a wall of squares.
    ///
    /// [ADR-0015]: ../../../docs/adr/0015-the-admin-console-reads-one-row-at-a-time.md
    #[tokio::test]
    async fn a_role_page_is_the_row_and_a_loop_page_is_the_column() {
        let box_of = ABox::already_administered().await;
        let held = box_of.signed_in_as("root").await;
        let role = box_of.a_role_called(&held, "Flight Director").await;
        box_of.a_ruled_on_loop_called(&held, "GNC").await;
        let flight = box_of.a_ruled_on_loop_called(&held, "FLIGHT").await;
        let set = box_of.sets(&held, &role, &flight, "control").await;
        assert_eq!(set.status, StatusCode::OK, "{:?}", set.body);

        let row = box_of
            .get_holding(&held, &format!("/api/roles/{role}/grid"))
            .await;
        assert_eq!(
            names_in(&row.body),
            ["Flight Director", "GNC", "FLIGHT"],
            "a role's row was not every loop in the base order: {:?}",
            row.body
        );
        assert_eq!(permissions_in(&row.body), ["none", "control"]);

        let column = box_of
            .get_holding(&held, &format!("/api/loops/{flight}/grid"))
            .await;
        assert_eq!(
            names_in(&column.body),
            ["FLIGHT", "Flight Director", "Observer"],
            "a loop's column was not every role by name: {:?}",
            column.body
        );
        assert_eq!(permissions_in(&column.body), ["control", "none"]);
    }

    /// Taking a permission away is setting `none`, and it is the same write as granting one.
    #[tokio::test]
    async fn a_cell_holds_one_value_and_taking_it_away_is_setting_none() {
        let box_of = ABox::already_administered().await;
        let held = box_of.signed_in_as("root").await;
        let role = box_of.a_role_called(&held, "Flight Director").await;
        let flight = box_of.a_ruled_on_loop_called(&held, "FLIGHT").await;

        box_of.sets(&held, &role, &flight, "emit").await;
        let taken_away = box_of.sets(&held, &role, &flight, "none").await;

        assert_eq!(taken_away.status, StatusCode::OK, "{:?}", taken_away.body);
        assert_eq!(permissions_in(&taken_away.body), ["none"]);
    }

    /// Cell edits are audited with before and after (v1 §12), like every other configuration
    /// write. The entry names the loop and reads by the pair.
    #[tokio::test]
    async fn setting_a_cell_is_audited_with_before_and_after() {
        let box_of = ABox::already_administered().await;
        let held = box_of.signed_in_as("root").await;
        let role = box_of.a_role_called(&held, "Flight Director").await;
        let flight = box_of.a_ruled_on_loop_called(&held, "FLIGHT").await;

        box_of.sets(&held, &role, &flight, "emit").await;
        box_of.sets(&held, &role, &flight, "control").await;

        let entries = box_of.entries().await;
        assert_eq!(entries[0].event, AuditEvent::GridCellEdited);
        let write = entries[0].write.as_ref().expect("a configuration write");
        assert_eq!(
            write.target.as_ref().map(|target| target.as_str()),
            Some(flight.as_str()),
            "the entry did not name the loop the authority is over"
        );
        assert_eq!(write.target_name, "Flight Director on FLIGHT");
        assert_eq!(
            write.before.as_ref().map(Snapshot::as_str),
            Some("role=Flight Director loop=FLIGHT permission=emit")
        );
        assert_eq!(
            write.after.as_ref().map(Snapshot::as_str),
            Some("role=Flight Director loop=FLIGHT permission=control")
        );
        assert_eq!(
            write.blast_radius,
            crate::configuration::BlastRadius::nothing_live(),
            "no session exists, so nothing live was touched"
        );
    }

    /// `unreviewed` is cleared **per loop, not per cell**, and dismissing it records a
    /// deliberate `none` for every role nobody ruled on (v1 §9).
    #[tokio::test]
    async fn dismissing_unreviewed_is_per_loop_and_records_deliberate_nones() {
        let box_of = ABox::already_administered().await;
        let held = box_of.signed_in_as("root").await;
        box_of.a_role_called(&held, "Flight Director").await;
        let flight = box_of.a_loop_called(&held, "FLIGHT").await;
        let gnc = box_of.a_loop_called(&held, "GNC").await;

        let dismissed = box_of
            .post_holding(
                &held,
                &format!("/api/loops/{flight}/dismiss-unreviewed"),
                "",
            )
            .await;

        assert_eq!(dismissed.status, StatusCode::OK, "{:?}", dismissed.body);
        assert!(
            dismissed.body.contains(r#""unreviewed":false"#),
            "the mark was not cleared: {:?}",
            dismissed.body
        );
        assert!(
            box_of
                .get_holding(&held, &format!("/api/loops/{gnc}"))
                .await
                .body
                .contains(r#""unreviewed":true"#),
            "dismissing one loop's mark cleared another's"
        );

        let column = box_of
            .get_holding(&held, &format!("/api/loops/{flight}/grid"))
            .await;
        assert_eq!(
            permissions_in(&column.body),
            ["none", "none"],
            "the roles nobody ruled on were not recorded"
        );

        let entries = box_of.entries().await;
        assert_eq!(entries[0].event, AuditEvent::LoopReviewed);
        let write = entries[0].write.as_ref().expect("a configuration write");
        assert_eq!(
            write.before.as_ref().map(Snapshot::as_str),
            Some("loop=FLIGHT reviewed=no")
        );
        assert_eq!(
            write.after.as_ref().map(Snapshot::as_str),
            Some("loop=FLIGHT reviewed=yes")
        );
    }

    /// The matrix survives as a **secondary reference view** ([ADR-0015]): a whole read, and
    /// nothing writes through it.
    ///
    /// [ADR-0015]: ../../../docs/adr/0015-the-admin-console-reads-one-row-at-a-time.md
    #[tokio::test]
    async fn the_matrix_is_a_reference_view_that_nothing_writes_through() {
        let box_of = ABox::already_administered().await;
        let held = box_of.signed_in_as("root").await;
        let role = box_of.a_role_called(&held, "Flight Director").await;
        let flight = box_of.a_ruled_on_loop_called(&held, "FLIGHT").await;
        box_of.a_ruled_on_loop_called(&held, "GNC").await;
        box_of.sets(&held, &role, &flight, "monitor").await;

        let matrix = box_of.get_holding(&held, "/api/grid").await;

        assert_eq!(matrix.status, StatusCode::OK, "{:?}", matrix.body);
        assert_eq!(
            names_in(&matrix.body),
            ["Flight Director", "Observer", "FLIGHT", "GNC"],
            "the matrix did not carry both axes: {:?}",
            matrix.body
        );
        assert_eq!(
            permissions_in(&matrix.body),
            ["monitor", "none", "none", "none"],
            "the matrix is not every role against every loop: {:?}",
            matrix.body
        );

        for method in ["POST", "PUT", "PATCH", "DELETE"] {
            let answer = box_of.holding(&held, method, "/api/grid", "{}").await;

            assert_eq!(
                answer.status,
                StatusCode::METHOD_NOT_ALLOWED,
                "the reference view answered a {method}"
            );
        }
    }

    /// A permission is one of an ordered four and there is no fifth word for one, so a body
    /// naming something else is a request that cannot be carried out rather than a refusal —
    /// and, like an edit that asks for no change, there is no write for an entry to be about.
    #[tokio::test]
    async fn a_permission_that_is_not_one_of_the_four_is_refused_rather_than_audited() {
        let box_of = ABox::already_administered().await;
        let held = box_of.signed_in_as("root").await;
        let role = box_of.a_role_called(&held, "Flight Director").await;
        let flight = box_of.a_ruled_on_loop_called(&held, "FLIGHT").await;

        let answer = box_of.sets(&held, &role, &flight, "administrator").await;

        assert_eq!(answer.status, StatusCode::BAD_REQUEST, "{:?}", answer.body);
        assert!(
            !box_of
                .audited()
                .await
                .iter()
                .any(|(event, ..)| *event == AuditEvent::GridCellEdited),
            "a request that made no write was audited as one"
        );
    }

    #[tokio::test]
    async fn a_cell_naming_a_role_or_a_loop_nobody_holds_says_so() {
        let box_of = ABox::already_administered().await;
        let held = box_of.signed_in_as("root").await;
        let role = box_of.a_role_called(&held, "Flight Director").await;
        let flight = box_of.a_ruled_on_loop_called(&held, "FLIGHT").await;

        for (role, on) in [
            (role.as_str(), "no-such-loop"),
            ("no-such-role", flight.as_str()),
        ] {
            let answer = box_of.sets(&held, role, on, "emit").await;

            assert_eq!(answer.status, StatusCode::NOT_FOUND, "{:?}", answer.body);
        }
        assert_eq!(
            box_of
                .get_holding(&held, "/api/roles/no-such-role/grid")
                .await
                .status,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            box_of
                .get_holding(&held, "/api/loops/no-such-loop/grid")
                .await
                .status,
            StatusCode::NOT_FOUND
        );
    }

    /// The grid is `SystemAdministration` like every other configuration surface, and a
    /// refused write is audited while a refused read is not (v1 §3).
    #[tokio::test]
    async fn nobody_but_a_system_administrator_may_read_or_write_the_grid() {
        let box_of = ABox::already_administered().await;
        let held = box_of.signed_in_as("root").await;
        let role = box_of.a_role_called(&held, "Flight Director").await;
        let flight = box_of.a_ruled_on_loop_called(&held, "FLIGHT").await;
        box_of.a_user_who_can_sign_in("flight", false).await;
        let theirs = box_of.signed_in_as("flight").await;

        assert_eq!(
            box_of.sets(&theirs, &role, &flight, "control").await.status,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            box_of
                .post_holding(
                    &theirs,
                    &format!("/api/loops/{flight}/dismiss-unreviewed"),
                    ""
                )
                .await
                .status,
            StatusCode::FORBIDDEN
        );
        for path in [
            "/api/grid".to_owned(),
            format!("/api/roles/{role}/grid"),
            format!("/api/loops/{flight}/grid"),
        ] {
            assert_eq!(
                box_of.get_holding(&theirs, &path).await.status,
                StatusCode::FORBIDDEN
            );
        }

        let refused = box_of
            .entries()
            .await
            .into_iter()
            .filter(|entry| entry.event == AuditEvent::AdministrationRefused)
            .count();
        assert_eq!(
            refused, 2,
            "a refused write was not audited, or a refused read was"
        );
        assert_eq!(
            permissions_in(
                &box_of
                    .get_holding(&held, &format!("/api/roles/{role}/grid"))
                    .await
                    .body
            ),
            ["none"],
            "a refused write landed anyway"
        );
    }
}
