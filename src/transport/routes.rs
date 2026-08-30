//! Route registration, and the rule that every route carries its authorisation requirement.
//!
//! There is one way to register a route and it takes a [`Requirement`] as a mandatory
//! positional argument. There is no default and no builder step that can be skipped, so a
//! route nobody ruled on does not compile — which is the entire mechanism of [ADR-0054]. A
//! reviewer sees `Requirement::Public` typed out in the diff, or they see a build failure.
//!
//! If this is ever softened into a default, ADR-0054 is void.
//!
//! The requirement is evaluated **per request**, not at registration and not at the socket
//! upgrade, and the caller it resolves to is read from the store every time (v1 §3).
//!
//! [ADR-0054]: ../../../docs/adr/0054-every-operation-declares-its-authorisation.md

use std::sync::Arc;

use axum::Router;
use axum::extract::Request;
use axum::extract::connect_info::IntoMakeServiceWithConnectInfo;
use axum::handler::Handler;
use axum::middleware::Next;
use axum::response::Response;
use axum::routing::{MethodRouter, any, delete, get, patch, post};
use tracing::Instrument;

use super::{answers, cookies};
use crate::authorisation::{self, Outcome, Requirement};
use crate::configuration::Store;
use crate::telemetry::module;

/// Every route the server answers on, each with the requirement it was registered under.
pub(super) struct RouteTable<S = ()> {
    router: Router<S>,
    /// The route that answers everything else, held until the table is sealed: a fallback
    /// takes a service rather than a router, and a service is what it becomes once the state
    /// it was registered against is known.
    fallback: Option<MethodRouter<S>>,
    store: Arc<Store>,
}

impl<S> RouteTable<S>
where
    S: Clone + Send + Sync + 'static,
{
    pub(super) fn new(store: Arc<Store>) -> Self {
        Self {
            router: Router::new(),
            fallback: None,
            store,
        }
    }

    /// Register a `GET`, under `requirement`.
    pub(super) fn get<H, T>(self, path: &str, requirement: Requirement, handler: H) -> Self
    where
        H: Handler<T, S>,
        T: 'static,
    {
        self.route(path, requirement, get(handler))
    }

    /// Register a `POST`, under `requirement`.
    pub(super) fn post<H, T>(self, path: &str, requirement: Requirement, handler: H) -> Self
    where
        H: Handler<T, S>,
        T: 'static,
    {
        self.route(path, requirement, post(handler))
    }

    /// Register a `PATCH`, under `requirement`.
    pub(super) fn patch<H, T>(self, path: &str, requirement: Requirement, handler: H) -> Self
    where
        H: Handler<T, S>,
        T: 'static,
    {
        self.route(path, requirement, patch(handler))
    }

    /// Register a `DELETE`, under `requirement`.
    pub(super) fn delete<H, T>(self, path: &str, requirement: Requirement, handler: H) -> Self
    where
        H: Handler<T, S>,
        T: 'static,
    {
        self.route(path, requirement, delete(handler))
    }

    /// Register the route that answers everything else, under `requirement`.
    ///
    /// The fallback is a route like any other and is ruled on like any other: the client
    /// bundle is served from it, and serving the bundle is `Public`.
    pub(super) fn fallback<H, T>(mut self, requirement: Requirement, handler: H) -> Self
    where
        H: Handler<T, S>,
        T: 'static,
    {
        self.fallback = Some(guarded(requirement, self.store.clone(), any(handler)));
        self
    }

    /// Hand the table to the server, with the backstop closed behind it.
    ///
    /// The router itself never leaves this module. Once it is out, `.route()` on it takes no
    /// requirement and compiles, and the guarantee above is only as good as whoever reads
    /// the next diff.
    ///
    /// The connection's peer address goes with it, because that is what the rate limits key
    /// on ([ADR-0025]) — and it is the true source, since VoxLoop terminates TLS itself with
    /// no proxy in front of it to be lied to.
    ///
    /// [ADR-0025]: ../../../docs/adr/0025-credentials-are-administered-because-there-is-no-email.md
    pub(super) fn into_make_service(
        self,
        state: S,
    ) -> IntoMakeServiceWithConnectInfo<Router, std::net::SocketAddr> {
        self.sealed(state)
            .into_make_service_with_connect_info::<std::net::SocketAddr>()
    }

    fn sealed(self, state: S) -> Router {
        let mut router = self.router;

        if let Some(fallback) = self.fallback {
            router = router.fallback_service(fallback.with_state(state.clone()));
        }

        router
            .layer(axum::middleware::from_fn(refuse_the_unruled))
            .with_state(state)
    }

    /// Ask the table for an answer, the way the server would.
    ///
    /// Tests reach the routes through this rather than through a bare router, so that what
    /// they exercise is what the server serves — the backstop included.
    #[cfg(test)]
    pub(super) async fn answer_to(self, state: S, path: &str) -> (axum::http::StatusCode, String) {
        let answer = self
            .answer(
                state,
                axum::http::Request::builder()
                    .uri(path)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await;
        let status = answer.status();
        let body = axum::body::to_bytes(answer.into_body(), usize::MAX)
            .await
            .expect("a body");

        (status, String::from_utf8_lossy(&body).into_owned())
    }

    /// Answer one request the way the server would, headers and all.
    #[cfg(test)]
    pub(super) async fn answer(
        self,
        state: S,
        request: axum::http::Request<axum::body::Body>,
    ) -> Response {
        use tower::ServiceExt;

        self.sealed(state)
            .oneshot(request)
            .await
            .expect("a response")
    }

    fn route(
        mut self,
        path: &str,
        requirement: Requirement,
        method_router: MethodRouter<S>,
    ) -> Self {
        let guarded = guarded(requirement, self.store.clone(), method_router);
        self.router = self.router.route(path, guarded);
        self
    }
}

/// Put the requirement in front of the handler, so nothing reaches a handler unruled.
fn guarded<S>(
    requirement: Requirement,
    store: Arc<Store>,
    method_router: MethodRouter<S>,
) -> MethodRouter<S>
where
    S: Clone + Send + Sync + 'static,
{
    method_router.layer(axum::middleware::from_fn(
        move |mut request: Request, next: Next| {
            let requirement = requirement.clone();
            let store = Arc::clone(&store);
            async move {
                let span = tracing::info_span!(
                    target: module::TRANSPORT,
                    "request",
                    method = %request.method(),
                    path = %request.uri().path()
                );

                let presented = cookies::presented(&axum_extra::extract::CookieJar::from_headers(
                    request.headers(),
                ));

                let mut answer =
                    match authorisation::evaluate(&requirement, presented, &store).await {
                        Ok(Outcome::Permitted(caller)) => {
                            // The handler acts on whoever the store said this was, this
                            // request. Nothing downstream re-reads the cookie.
                            request.extensions_mut().insert(caller);
                            next.run(request).instrument(span).await
                        }
                        Ok(Outcome::Refused) => {
                            let _entered = span.enter();
                            tracing::debug!(target: module::AUTHORISATION, ?requirement, "refused");
                            answers::refusal(unmet(&requirement))
                        }
                        Err(error) => {
                            let _entered = span.enter();
                            answers::unavailable(&error)
                        }
                    };

                answer.extensions_mut().insert(Ruled);
                answer
            }
        },
    ))
}

/// What a caller is missing, said plainly enough to act on.
///
/// Authorisation answers permitted or refused and never *why*; turning that into something a
/// human reads is Transport's job, and this is where it happens.
fn unmet(requirement: &Requirement) -> &'static str {
    match requirement {
        // Nothing public is refused for want of a principal, so this arm answers a question
        // nobody asked — and saying so is better than inventing a reason.
        Requirement::Public => "That operation is not available.",
        Requirement::SignedIn => "That operation is for a signed-in user.",
        Requirement::Session => "That operation is for a user who has assumed a role.",
        Requirement::SystemAdministration => "That operation is for a system administrator.",
        Requirement::ServiceToken => "That operation is for a service principal.",
    }
}

/// Left on every answer that came through [`guarded`], and on no other.
#[derive(Clone, Copy)]
struct Ruled;

/// The backstop behind the mechanism ([ADR-0054]): an answer that was never ruled on is
/// refused here rather than sent.
///
/// The mechanism is [`RouteTable`] itself — a requirement is an argument, so an unruled route
/// does not compile. This catches the one case the compiler cannot: a route reaching the
/// router by some path other than `RouteTable`.
///
/// [ADR-0054]: ../../../docs/adr/0054-every-operation-declares-its-authorisation.md
async fn refuse_the_unruled(request: Request, next: Next) -> Response {
    let answer = next.run(request).await;

    if answer.extensions().get::<Ruled>().is_some() {
        return answer;
    }

    tracing::error!(target: module::TRANSPORT, "an unruled route answered; refusing it");
    answers::refusal("That operation is not available.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::a_temporary_store;
    use axum::http::StatusCode;

    async fn answer() -> &'static str {
        "answered"
    }

    async fn a_table() -> (tempfile::TempDir, RouteTable) {
        let (directory, store) = a_temporary_store().await;
        (directory, RouteTable::new(Arc::new(store)))
    }

    async fn status_of(table: RouteTable, path: &str) -> StatusCode {
        table.answer_to((), path).await.0
    }

    #[tokio::test]
    async fn a_route_declared_public_answers() {
        let (_directory, table) = a_table().await;
        let table = table.get("/open", Requirement::Public, answer);

        assert_eq!(status_of(table, "/open").await, StatusCode::OK);
    }

    #[tokio::test]
    async fn a_route_declaring_a_requirement_nobody_presents_is_refused() {
        for requirement in [
            Requirement::SignedIn,
            Requirement::Session,
            Requirement::SystemAdministration,
            Requirement::ServiceToken,
        ] {
            let (_directory, table) = a_table().await;
            let table = table.get("/shut", requirement.clone(), answer);

            assert_eq!(
                status_of(table, "/shut").await,
                StatusCode::FORBIDDEN,
                "expected {requirement:?} to be refused"
            );
        }
    }

    #[tokio::test]
    async fn a_refusal_says_why_rather_than_hiding_the_operation() {
        let (_directory, table) = a_table().await;
        let table = table.get("/shut", Requirement::SignedIn, answer);

        let (status, body) = table.answer_to((), "/shut").await;

        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(body.starts_with("You may not."), "got {body:?}");
        assert!(body.contains("signed-in"), "got {body:?}");
    }

    #[tokio::test]
    async fn a_route_that_reached_the_router_without_a_requirement_is_refused() {
        let (_directory, store) = a_temporary_store().await;
        // The only way to build one: the compiler stops every other route from getting here.
        let sneaked = RouteTable {
            router: Router::new().route("/sneaked", get(answer)),
            fallback: None,
            store: Arc::new(store),
        };

        assert_eq!(status_of(sneaked, "/sneaked").await, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn the_fallback_carries_a_requirement_like_any_other_route() {
        let (_directory, table) = a_table().await;
        let table = table.fallback(Requirement::SignedIn, answer);

        assert_eq!(
            status_of(table, "/anything-at-all").await,
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn a_method_the_route_does_not_take_is_still_answered_honestly() {
        use axum::body::Body;
        use tower::ServiceExt;

        let (_directory, table) = a_table().await;
        let table = table.get("/open", Requirement::Public, answer);

        let answer = table
            .sealed(())
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/open")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("a response");

        assert_eq!(answer.status(), StatusCode::METHOD_NOT_ALLOWED);
    }
}
