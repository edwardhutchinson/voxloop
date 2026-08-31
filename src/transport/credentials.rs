//! What a request presents to say who is making it, and the rule that it is one thing.
//!
//! **A request carries exactly one credential kind** (v1 §3). Reading both here is what lets
//! Authorisation refuse a request that carries two rather than resolve it by precedence — a
//! server that never looks for a token cannot refuse one alongside a cookie.
//!
//! The token rides an `Authorization` header and **never a query string** ([ADR-0026]): a
//! query string is in every access log, every referrer and every screenshot of a browser's
//! address bar, and a standing credential with no expiry is the last thing that belongs in
//! one.
//!
//! [ADR-0026]: ../../../docs/adr/0026-one-credential-and-the-media-path-carries-none.md

use axum::http::HeaderMap;
use axum_extra::extract::CookieJar;

use super::cookies;
use crate::authorisation::ServiceToken;
use crate::configuration::SignInToken;

/// Whatever this request offered, before anything has been resolved.
pub(super) struct Credentials {
    /// What a browser presents: the sign-in cookie, carrying no claims.
    pub(super) sign_in: Option<SignInToken>,
    /// What a script presents: a service principal's bearer token.
    pub(super) service_token: Option<ServiceToken>,
}

/// Read what the request presented.
pub(super) fn presented(headers: &HeaderMap) -> Credentials {
    Credentials {
        sign_in: cookies::presented(&CookieJar::from_headers(headers)),
        service_token: bearer(headers),
    }
}

/// The bearer token in an `Authorization` header, if the header carries one.
///
/// Only `Bearer` is read. Another scheme is not a VoxLoop credential and is left where it
/// is: treating it as one would refuse a request for presenting something this deployment
/// does not issue.
fn bearer(headers: &HeaderMap) -> Option<ServiceToken> {
    let offered = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let (scheme, token) = offered.split_once(' ')?;

    scheme
        .eq_ignore_ascii_case("bearer")
        .then(|| ServiceToken::presented(token.trim().to_owned()))
        .filter(|_| !token.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(name: &'static str, value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(name, value.parse().expect("a header value"));
        headers
    }

    #[test]
    fn reads_the_sign_in_a_browser_presents() {
        let presented = presented(&headers("cookie", "voxloop_sign_in=a-token"));

        assert_eq!(
            presented.sign_in.expect("a sign-in").as_str(),
            "a-token",
            "the cookie was not read"
        );
        assert!(presented.service_token.is_none());
    }

    #[test]
    fn reads_the_token_a_script_presents_in_an_authorization_header() {
        let presented = presented(&headers("authorization", "Bearer a-service-token"));

        assert!(presented.service_token.is_some());
        assert!(presented.sign_in.is_none());
    }

    /// Both is what Authorisation refuses, so both is what this has to be able to say.
    #[test]
    fn reads_both_where_a_request_presents_both() {
        let mut both = headers("cookie", "voxloop_sign_in=a-token");
        both.insert(
            "authorization",
            "Bearer a-service-token".parse().expect("a header value"),
        );

        let presented = presented(&both);

        assert!(presented.sign_in.is_some() && presented.service_token.is_some());
    }

    #[test]
    fn reads_no_token_from_a_scheme_voxloop_does_not_issue() {
        for offered in ["Basic abcdef", "Bearer", "Bearer  ", "nonsense"] {
            assert!(
                presented(&headers("authorization", offered))
                    .service_token
                    .is_none(),
                "{offered:?} was read as a service token"
            );
        }
    }
}
