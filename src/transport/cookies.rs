//! The one cookie VoxLoop sets, and what is deliberately not in it.
//!
//! **The cookie carries no claims** (v1 §3): not the username, not the system-administration
//! flag, not the assumed role. It holds an opaque sign-in token and nothing else, so every
//! fact about the caller is read from the store per request and revocation is immediate
//! rather than eventual.
//!
//! It is `Secure`, which is why HTTPS is mandatory even on a LAN — a `Secure` cookie is not
//! sent otherwise, so a VoxLoop without TLS has no sign-in at all ([ADR-0040]).
//!
//! [ADR-0040]: ../../../docs/adr/0040-one-binary-one-unit-four-moving-parts.md

use axum_extra::extract::CookieJar;
use axum_extra::extract::cookie::{Cookie, SameSite};

use crate::configuration::SignInToken;

/// The name the browser files it under.
const SIGN_IN: &str = "voxloop_sign_in";

/// The cookie that says this browser is signed in.
pub(super) fn holds(token: &SignInToken) -> Cookie<'static> {
    let mut cookie = Cookie::new(SIGN_IN, token.as_str().to_owned());

    // `Secure` and `HttpOnly` keep it off the wire in clear and out of reach of script;
    // `Strict` keeps another site from spending it; the root path is what makes one cookie
    // serve the console, the API and the signalling upgrade alike.
    cookie.set_secure(true);
    cookie.set_http_only(true);
    cookie.set_same_site(SameSite::Strict);
    cookie.set_path("/");

    cookie
}

/// The cookie that says this browser is not.
///
/// Signing out ends the sign-in in the store; this is the browser's copy being taken back,
/// so that a stale value cannot be presented against a sign-in somebody else opened later.
pub(super) fn taken_back() -> Cookie<'static> {
    let mut cookie = holds(&SignInToken::presented(String::new()));
    cookie.make_removal();
    cookie
}

/// The sign-in this request presented, if it presented one.
pub(super) fn presented(jar: &CookieJar) -> Option<SignInToken> {
    jar.get(SIGN_IN)
        .map(|cookie| SignInToken::presented(cookie.value().to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hands_the_browser_a_cookie_that_carries_the_token_and_nothing_else() {
        let cookie = holds(&SignInToken::presented("a-token".to_owned()));

        assert_eq!(cookie.value(), "a-token");
        assert_eq!(cookie.secure(), Some(true));
        assert_eq!(cookie.http_only(), Some(true));
        assert_eq!(cookie.same_site(), Some(SameSite::Strict));
        assert_eq!(cookie.path(), Some("/"));
    }

    #[test]
    fn reads_back_the_token_a_browser_presents() {
        let jar = CookieJar::new().add(holds(&SignInToken::presented("a-token".to_owned())));

        assert_eq!(presented(&jar).expect("a token").as_str(), "a-token");
    }

    #[test]
    fn reads_no_token_from_a_browser_that_presents_none() {
        assert!(presented(&CookieJar::new()).is_none());
    }

    #[test]
    fn taking_the_cookie_back_expires_it_and_empties_it() {
        let cookie = taken_back();

        assert_eq!(cookie.value(), "");
        assert!(
            cookie.to_string().contains("Max-Age=0"),
            "the cookie was not expired: {cookie}"
        );
    }
}
