//! Redeeming an enrolment code: an account that cannot be signed into becomes one that can.
//!
//! This is the whole of what replaces the link an email would have carried ([ADR-0025]). The
//! code was issued by an administrator against a user and handed over out of band, so **the
//! code identifies the user** and there is nothing else to present: no username to submit,
//! nothing to enumerate, and no way to aim a redemption at somebody else's account.
//!
//! It is `Public` because a user with no password has no way to be anything else. A reset is
//! this same route again, which is why **there is no self-service reset here and no
//! self-registration anywhere**: a code is the only way in, and only an administrator issues
//! one.
//!
//! [ADR-0025]: ../../../docs/adr/0025-credentials-are-administered-because-there-is-no-email.md

use std::net::SocketAddr;

use axum::Json;
use axum::extract::{ConnectInfo, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use super::{Api, answers, name_as_it_stands, unstorable};
use crate::configuration::{
    AuditEntry, AuditEvent, AuditLog, Enrolment, EnrolmentCode, SignIns, StoreError, Users,
};
use crate::telemetry::module;

/// What somebody holding a code presents to set their password.
#[derive(Deserialize)]
pub(super) struct Redemption {
    code: String,
    password: String,
}

/// A presented code is a live credential, and one that turns up in a log is spent.
impl std::fmt::Debug for Redemption {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Redemption { code: withheld, password: withheld }")
    }
}

/// Redeem an enrolment code. `Public`, rate-limited on source, audited either way.
pub(super) async fn redeem(
    State(api): State<Api>,
    ConnectInfo(source): ConnectInfo<SocketAddr>,
    Json(presented): Json<Redemption>,
) -> Response {
    if !api.admits(&source) {
        return answers::too_many_attempts();
    }

    answers::or_unavailable(enrol(&api, presented, &source).await)
}

async fn enrol(
    api: &Api,
    presented: Redemption,
    source: &SocketAddr,
) -> Result<Response, StoreError> {
    let mut transaction = api.store.begin().await?;

    // The code is spent first, and the transaction is what makes that safe: everything after
    // this point that refuses rolls back, and the code is still good. Checking without
    // spending would be two statements and a window in which one code enrols twice.
    let code = EnrolmentCode::presented(presented.code);
    let Some(user) = transaction.spend_enrolment_code(&code).await? else {
        transaction.roll_back().await?;
        return refuse(api, source).await;
    };

    // Hashing here holds a pooled connection for as long as Argon2id takes, which is the
    // trade `sign_in` makes for the same reason: the alternative is a seam that hands the
    // work outside Identity. The rate limits are what bound the cost.
    //
    // The code survives a password VoxLoop would not store, because the rollback puts it
    // back. Spending it here would cost somebody their only way in over a typo, and the
    // administrator who issued it is not necessarily in the building.
    let hashed = match api.identity.hash_password(&presented.password) {
        Ok(hashed) => hashed,
        Err(refusal) => {
            transaction.roll_back().await?;
            return Ok(unstorable(&refusal));
        }
    };

    let Some(_) = transaction.set_password(&user, hashed).await? else {
        // Unreachable: a code that enrols somebody is a row pointing at a user record.
        transaction.roll_back().await?;
        return Ok(answers::no_such("user"));
    };

    // The credential this account had a moment ago is not the one it has now, so nothing
    // standing against the old one is left standing. In the ordinary flow this ends nothing:
    // a forced reset has already done it, or the account never had a password to sign in
    // with. It is the flow where an administrator issues a code without forcing a reset that
    // this covers, and leaving a sign-in open against a replaced credential is the worse of
    // the two answers. Changing one's own password is the deliberate opposite (v1 §2).
    transaction.end_every_sign_in(&user).await?;

    let name = name_as_it_stands(&mut transaction, &user).await?;
    transaction
        .record(AuditEntry {
            event: AuditEvent::EnrolmentRedeemed,
            // The user is the actor: proving possession of the code is the whole of what a
            // redemption establishes, and it establishes it about them. Issuing the code was
            // the administrator's act and is its own entry.
            actor: Some(user.clone()),
            actor_name: name,
            source: Some(source.ip()),
            // An act by a principal on their own credential rather than a configuration
            // write on somebody's record, which is the line every entry here is on: the
            // administrator issuing the code is the write.
            write: None,
            operation: None,
        })
        .await?;
    transaction.commit().await?;

    tracing::info!(
        target: module::IDENTITY,
        user = %user.as_str(),
        "an enrolment code was redeemed"
    );

    Ok(StatusCode::NO_CONTENT.into_response())
}

/// Refuse a code, and record that somebody tried.
///
/// A code that was never issued, one already spent and one that has expired are one answer,
/// because which of the three it was is not something an unauthenticated caller is entitled
/// to learn. The entry names nobody for the same reason there is nobody to name: a code that
/// enrols nobody says nothing about who presented it.
async fn refuse(api: &Api, source: &SocketAddr) -> Result<Response, StoreError> {
    tracing::warn!(
        target: module::IDENTITY,
        source = %source.ip(),
        "an enrolment code was presented and refused"
    );

    let mut transaction = api.store.begin().await?;
    transaction
        .record(AuditEntry {
            event: AuditEvent::EnrolmentRefused,
            actor: None,
            actor_name: String::new(),
            source: Some(source.ip()),
            write: None,
            operation: None,
        })
        .await?;
    transaction.commit().await?;

    Ok(answers::refusal(
        "That enrolment code is not one this deployment is holding. Ask an administrator for \
         another.",
    ))
}
