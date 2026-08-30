//! Authorisation — evaluates one requirement against a caller and answers permitted or
//! refused.
//!
//! Nothing here says *why* in a form the caller can act on. Transport turns a refusal into
//! something a human reads; this module answers the question and nothing more.
//!
//! Every operation carries exactly one [`Requirement`], typed at the point it is registered
//! ([ADR-0054]). There is no default value and no way to register an operation without one,
//! so an operation nobody ruled on is a build failure rather than an open door.
//!
//! **The cookie carries no claims** (v1 §3). Neither the system-administration flag nor the
//! assumed role is in it, so everything about the caller is read here, from the store, on
//! every request. That is what makes revocation immediate rather than eventual.
//!
//! [ADR-0054]: ../../docs/adr/0054-every-operation-declares-its-authorisation.md

use crate::configuration::{SignInToken, SignIns, Store, StoreError, UserId, Users};

/// What an operation demands of whoever calls it.
///
/// ADR-0054 fixes six requirements and no seventh. Five of them are a function of the caller
/// alone and are named here. The sixth, `Grid(rung, loop)`, is a function of the operation's
/// *arguments* as well — it names a loop the caller has yet to supply — and every operation
/// carrying it is a signalling-channel message rather than an HTTP route
/// (`docs/spec/api-surface.md`). It arrives with the socket and the grid, alongside the loop
/// identity and the four rungs it needs, and it is deliberately not invented here.
// Two of the five name something no principal can hold yet: a role (#37) or a service token
// (#57). They are declared together anyway: the list is fixed by ADR-0054, not grown one
// route at a time.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Requirement {
    /// No principal at all: the client bundle, sign-in, redemption, liveness.
    Public,
    /// An authenticated user who has assumed no role.
    SignedIn,
    /// A user who has assumed a role.
    Session,
    /// The user-level flag of ADR-0003, held by the person and never by a role.
    SystemAdministration,
    /// A service principal, presenting its token in an `Authorization` header.
    ServiceToken,
}

/// Whoever is calling, as the store had them at the moment they called.
#[derive(Clone, Debug)]
pub(crate) enum Caller {
    /// Nobody authenticated, which is all a `Public` operation ever needs.
    Nobody,
    /// A signed-in user, resolved from the store this request.
    User {
        id: UserId,
        /// The sign-in they presented, which is what a sign-out ends.
        sign_in: SignInToken,
    },
}

/// The answer, and the whole of it.
#[derive(Clone, Debug)]
pub(crate) enum Outcome {
    /// The call may proceed, on behalf of this caller.
    Permitted(Caller),
    Refused,
}

/// Decide whether this call may proceed, and on whose behalf.
///
/// The store is passed rather than a transaction, and that is not the breach of ADR-0038 it
/// looks like: the rule there puts the handle in the caller's hands so that a write and its
/// audit entry commit together. This reads and writes nothing, so it has nothing to commit
/// with anybody — and requiring an open transaction here would mean opening one for every
/// static asset the console asks for.
pub(crate) async fn evaluate(
    requirement: &Requirement,
    presented: Option<SignInToken>,
    store: &Store,
) -> Result<Outcome, StoreError> {
    match requirement {
        Requirement::Public => Ok(Outcome::Permitted(Caller::Nobody)),

        Requirement::SignedIn => signed_in(presented, store, |_| true).await,

        // Gated on the user's flag and **never on a role** (v1 §9), which is why this arm
        // reads the same record `SignedIn` does and asks it one more question. An operator
        // who is also a sysadmin reaches the console without relinquishing, because there is
        // nothing here for a session to satisfy.
        //
        // A locked account is refused as well: locking ends the sign-in, so this is the
        // belt to that braces, and it costs nothing because the record is already read.
        Requirement::SystemAdministration => {
            signed_in(presented, store, |user| {
                user.is_system_administrator && !user.is_locked
            })
            .await
        }

        // A role is assumed over the signalling channel and a service principal is
        // administered, and neither exists yet. They are refused rather than waved through:
        // the default is refusal, everywhere and always.
        Requirement::Session | Requirement::ServiceToken => Ok(Outcome::Refused),
    }
}

/// Resolve the sign-in presented, and permit it where the user it names satisfies `holds`.
///
/// Everything about the caller is read here, from the store, on this request — the cookie
/// carries no claims (v1 §3) — which is what makes taking a flag away take effect now.
async fn signed_in(
    presented: Option<SignInToken>,
    store: &Store,
    holds: impl Fn(&crate::configuration::User) -> bool,
) -> Result<Outcome, StoreError> {
    let Some(token) = presented else {
        return Ok(Outcome::Refused);
    };

    let mut transaction = store.begin().await?;
    let resolved = async {
        let Some(id) = transaction.holder_of(&token).await? else {
            return Ok(None);
        };

        transaction.user(&id).await
    }
    .await;
    transaction.roll_back().await?;

    Ok(match resolved? {
        Some(user) if holds(&user) => Outcome::Permitted(Caller::User {
            id: user.id,
            sign_in: token,
        }),
        _ => Outcome::Refused,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::{NewUser, PasswordHash, Users, a_temporary_store};

    fn is_permitted(outcome: &Outcome) -> bool {
        matches!(outcome, Outcome::Permitted(_))
    }

    #[tokio::test]
    async fn a_public_operation_is_permitted_to_nobody_in_particular() {
        let (_directory, store) = a_temporary_store().await;

        let outcome = evaluate(&Requirement::Public, None, &store)
            .await
            .expect("an answer");

        assert!(matches!(outcome, Outcome::Permitted(Caller::Nobody)));
    }

    #[tokio::test]
    async fn a_signed_in_operation_is_permitted_to_whoever_the_store_says_holds_the_sign_in() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = transaction
            .create_user(NewUser {
                username: "flight".to_owned(),
                password_hash: None,
                is_system_administrator: false,
            })
            .await
            .expect("the user to be created");
        let token = transaction
            .open_sign_in(&user)
            .await
            .expect("the sign-in to open");
        transaction.commit().await.expect("the sign-in to land");

        let outcome = evaluate(&Requirement::SignedIn, Some(token), &store)
            .await
            .expect("an answer");

        let Outcome::Permitted(Caller::User { id, .. }) = outcome else {
            panic!("expected the sign-in to be permitted, got {outcome:?}");
        };
        assert_eq!(id, user);
    }

    #[tokio::test]
    async fn a_signed_in_operation_is_refused_to_a_caller_presenting_nothing() {
        let (_directory, store) = a_temporary_store().await;

        let outcome = evaluate(&Requirement::SignedIn, None, &store)
            .await
            .expect("an answer");

        assert!(!is_permitted(&outcome));
    }

    #[tokio::test]
    async fn a_signed_in_operation_is_refused_to_a_token_the_store_does_not_hold() {
        let (_directory, store) = a_temporary_store().await;

        let outcome = evaluate(
            &Requirement::SignedIn,
            Some(SignInToken::presented("guessed".to_owned())),
            &store,
        )
        .await
        .expect("an answer");

        assert!(!is_permitted(&outcome));
    }

    /// The flag comes from the store on every request, so ending a sign-in ends it now
    /// rather than when something expires.
    #[tokio::test]
    async fn a_sign_in_the_store_has_ended_stops_being_permitted_at_once() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = transaction
            .create_user(NewUser {
                username: "flight".to_owned(),
                password_hash: None,
                is_system_administrator: false,
            })
            .await
            .expect("the user to be created");
        let token = transaction
            .open_sign_in(&user)
            .await
            .expect("the sign-in to open");
        transaction.commit().await.expect("the sign-in to land");
        assert!(is_permitted(
            &evaluate(&Requirement::SignedIn, Some(token.clone()), &store)
                .await
                .expect("an answer")
        ));

        let mut transaction = store.begin().await.expect("a transaction");
        transaction
            .end_sign_in(&token)
            .await
            .expect("the sign-in to end");
        transaction.commit().await.expect("the sign-out to land");

        let outcome = evaluate(&Requirement::SignedIn, Some(token), &store)
            .await
            .expect("an answer");
        assert!(!is_permitted(&outcome));
    }

    /// The console opens on the flag alone and never on a role (v1 §9): this caller has
    /// assumed nothing, and an operator who is also a sysadmin must not have to drop off the
    /// air to administer the deployment.
    #[tokio::test]
    async fn system_administration_is_permitted_to_the_flag_holder_who_has_assumed_no_role() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = transaction
            .create_user(NewUser {
                username: "root".to_owned(),
                password_hash: None,
                is_system_administrator: true,
            })
            .await
            .expect("an administrator");
        let token = transaction
            .open_sign_in(&user)
            .await
            .expect("the sign-in to open");
        transaction.commit().await.expect("the sign-in to land");

        let outcome = evaluate(&Requirement::SystemAdministration, Some(token), &store)
            .await
            .expect("an answer");

        let Outcome::Permitted(Caller::User { id, .. }) = outcome else {
            panic!("expected the administrator to be permitted, got {outcome:?}");
        };
        assert_eq!(id, user);
    }

    #[tokio::test]
    async fn system_administration_is_refused_to_a_signed_in_user_without_the_flag() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = transaction
            .create_user(NewUser {
                username: "flight".to_owned(),
                password_hash: None,
                is_system_administrator: false,
            })
            .await
            .expect("an ordinary user");
        let token = transaction
            .open_sign_in(&user)
            .await
            .expect("the sign-in to open");
        transaction.commit().await.expect("the sign-in to land");

        let outcome = evaluate(&Requirement::SystemAdministration, Some(token), &store)
            .await
            .expect("an answer");

        assert!(!is_permitted(&outcome));
    }

    /// The cookie carries no claims, so the flag is read from the store on every request and
    /// taking it away stops the console now rather than when something expires (v1 §3).
    #[tokio::test]
    async fn taking_the_flag_away_stops_the_console_opening_at_once() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = transaction
            .create_user(NewUser {
                username: "root".to_owned(),
                password_hash: None,
                is_system_administrator: true,
            })
            .await
            .expect("an administrator");
        transaction
            .create_user(NewUser {
                username: "deputy".to_owned(),
                password_hash: Some(PasswordHash::already_hashed(
                    "$argon2id$stand-in".to_owned(),
                )),
                is_system_administrator: true,
            })
            .await
            .expect("a second administrator");
        let token = transaction
            .open_sign_in(&user)
            .await
            .expect("the sign-in to open");
        transaction.commit().await.expect("the sign-in to land");
        assert!(is_permitted(
            &evaluate(
                &Requirement::SystemAdministration,
                Some(token.clone()),
                &store
            )
            .await
            .expect("an answer")
        ));

        let mut transaction = store.begin().await.expect("a transaction");
        transaction
            .set_system_administration(&user, false)
            .await
            .expect("the flag to be cleared");
        transaction.commit().await.expect("the edit to land");

        let outcome = evaluate(&Requirement::SystemAdministration, Some(token), &store)
            .await
            .expect("an answer");
        assert!(!is_permitted(&outcome));
    }

    #[tokio::test]
    async fn every_requirement_no_principal_can_hold_yet_is_refused() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = transaction
            .create_user(NewUser {
                username: "root".to_owned(),
                password_hash: None,
                is_system_administrator: true,
            })
            .await
            .expect("an administrator");
        let token = transaction
            .open_sign_in(&user)
            .await
            .expect("the sign-in to open");
        transaction.commit().await.expect("the sign-in to land");

        for requirement in [Requirement::Session, Requirement::ServiceToken] {
            let outcome = evaluate(&requirement, Some(token.clone()), &store)
                .await
                .expect("an answer");

            assert!(
                !is_permitted(&outcome),
                "expected {requirement:?} to be refused"
            );
        }
    }
}
