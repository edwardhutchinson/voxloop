//! Route registration, and the rule that every route carries its authorisation requirement.
//!
//! There is one way to register a route and it takes a [`Requirement`] as a mandatory
//! positional argument. There is no default and no builder step that can be skipped, so a
//! route nobody ruled on does not compile — which is the entire mechanism of [ADR-0054]. A
//! reviewer sees `Requirement::Public` typed out in the diff, or they see a build failure.
//!
//! If this is ever softened into a default, ADR-0054 is void.
//!
//! [ADR-0054]: ../../../docs/adr/0054-every-operation-declares-its-authorisation.md

use axum::Router;
use axum::extract::Request;
use axum::handler::Handler;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{IntoMakeService, MethodRouter, any, get};
use tracing::Instrument;

use crate::authorisation::{self, Outcome, Requirement};
use crate::telemetry::module;

/// Every route the server answers on, each with the requirement it was registered under.
pub(super) struct RouteTable {
    router: Router,
}

impl RouteTable {
    pub(super) fn new() -> Self {
        Self {
            router: Router::new(),
        }
    }

    /// Register a `GET`, under `requirement`.
    pub(super) fn get<H, T>(self, path: &str, requirement: Requirement, handler: H) -> Self
    where
        H: Handler<T, ()>,
        T: 'static,
    {
        self.route(path, requirement, get(handler))
    }

    /// Register the route that answers everything else, under `requirement`.
    ///
    /// The fallback is a route like any other and is ruled on like any other: the client
    /// bundle is served from it, and serving the bundle is `Public`.
    pub(super) fn fallback<H, T>(mut self, requirement: Requirement, handler: H) -> Self
    where
        H: Handler<T, ()>,
        T: 'static,
    {
        self.router = self
            .router
            .fallback_service(guarded(requirement, any(handler)));
        self
    }

    /// Hand the table to the server, with the backstop closed behind it.
    ///
    /// The router itself never leaves this module. Once it is out, `.route()` on it takes no
    /// requirement and compiles, and the guarantee above is only as good as whoever reads
    /// the next diff.
    pub(super) fn into_make_service(self) -> IntoMakeService<Router> {
        self.sealed().into_make_service()
    }

    fn sealed(self) -> Router {
        self.router
            .layer(axum::middleware::from_fn(refuse_the_unruled))
    }

    /// Ask the table for an answer, the way the server would.
    ///
    /// Tests reach the routes through this rather than through a bare router, so that what
    /// they exercise is what the server serves — the backstop included.
    #[cfg(test)]
    pub(super) async fn answer_to(self, path: &str) -> (StatusCode, String) {
        use tower::ServiceExt;

        let answer = self
            .sealed()
            .oneshot(
                axum::http::Request::builder()
                    .uri(path)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .expect("a response");
        let status = answer.status();
        let body = axum::body::to_bytes(answer.into_body(), usize::MAX)
            .await
            .expect("a body");

        (status, String::from_utf8_lossy(&body).into_owned())
    }

    fn route(mut self, path: &str, requirement: Requirement, method_router: MethodRouter) -> Self {
        self.router = self.router.route(path, guarded(requirement, method_router));
        self
    }
}

/// Put the requirement in front of the handler, so nothing reaches a handler unruled.
fn guarded(requirement: Requirement, method_router: MethodRouter) -> MethodRouter {
    method_router.layer(axum::middleware::from_fn(
        move |request: Request, next: Next| {
            let requirement = requirement.clone();
            async move {
                let span = tracing::info_span!(
                    target: module::TRANSPORT,
                    "request",
                    method = %request.method(),
                    path = %request.uri().path()
                );

                let mut answer = match authorisation::evaluate(&requirement) {
                    Outcome::Permitted => next.run(request).instrument(span).await,
                    Outcome::Refused => {
                        let _entered = span.enter();
                        tracing::debug!(target: module::AUTHORISATION, ?requirement, "refused");
                        refusal()
                    }
                };

                answer.extensions_mut().insert(Ruled);
                answer
            }
        },
    ))
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
    refusal()
}

/// A refusal says *you may not*; it does not pretend the operation is not there.
///
/// Transport is the only module that may name an HTTP status, and this is where it does it.
fn refusal() -> Response {
    (StatusCode::FORBIDDEN, "You may not.").into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn answer() -> &'static str {
        "answered"
    }

    async fn status_of(table: RouteTable, path: &str) -> StatusCode {
        table.answer_to(path).await.0
    }

    #[tokio::test]
    async fn a_route_declared_public_answers() {
        let table = RouteTable::new().get("/open", Requirement::Public, answer);

        assert_eq!(status_of(table, "/open").await, StatusCode::OK);
    }

    #[tokio::test]
    async fn a_route_declaring_any_other_requirement_is_refused() {
        for requirement in [
            Requirement::SignedIn,
            Requirement::Session,
            Requirement::SystemAdministration,
            Requirement::ServiceToken,
        ] {
            let table = RouteTable::new().get("/shut", requirement.clone(), answer);

            assert_eq!(
                status_of(table, "/shut").await,
                StatusCode::FORBIDDEN,
                "expected {requirement:?} to be refused"
            );
        }
    }

    #[tokio::test]
    async fn a_route_that_reached_the_router_without_a_requirement_is_refused() {
        // The only way to build one: the compiler stops every other route from getting here.
        let sneaked = RouteTable {
            router: Router::new().route("/sneaked", get(answer)),
        };

        assert_eq!(status_of(sneaked, "/sneaked").await, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn the_fallback_carries_a_requirement_like_any_other_route() {
        let table = RouteTable::new().fallback(Requirement::SignedIn, answer);

        assert_eq!(
            status_of(table, "/anything-at-all").await,
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn a_method_the_route_does_not_take_is_still_answered_honestly() {
        use axum::body::Body;
        use tower::ServiceExt;

        let table = RouteTable::new().get("/open", Requirement::Public, answer);

        let answer = table
            .sealed()
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
