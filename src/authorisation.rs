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

use crate::configuration::{
    Grid, LoopId, Permission, RoleId, SignInToken, SignIns, Store, StoreError, UserId, Users,
};
use crate::state::{SessionId, StateAuthority};

/// What an operation demands of whoever calls it.
///
/// ADR-0054 fixes six requirements and no seventh. Five are a function of the caller alone.
/// The sixth, [`Requirement::Grid`], is a function of the operation's *arguments* as well —
/// it names a loop the caller supplies — so every operation carrying it is a
/// signalling-channel message rather than an HTTP route (`docs/spec/api-surface.md`), built
/// per message rather than registered once.
// One of the six names something no principal can hold yet: a service token (#57). It is
// declared anyway — the list is fixed by ADR-0054, not grown one route at a time.
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
    /// The acting principal's role holds at least `rung` on this loop.
    ///
    /// The acting principal is the one the operation is performed *as*: the assumed role for
    /// a user, the bound role for a service principal — never the role somebody is eligible
    /// for and not acting through, because a session is bound to exactly one role and reach
    /// is never composed across roles (v1 §1).
    ///
    /// Answering it is **one lookup** and there is nothing after it ([ADR-0011]). No
    /// per-user grant, no per-user deny, no override, no exception layer and no precedence
    /// rule — each of those would be a second lookup that could disagree with the first, and
    /// then a loop's column would never be the whole answer to *who may hear this*.
    ///
    /// `rung` is one of `monitor`, `emit` and `control` — the three the operations in
    /// `docs/spec/api-surface.md` ask for. `none` is expressible because a rung is a
    /// permission and a permission is the four, and it demands nothing of anybody: an
    /// operation wanting that is `Session`, and there is none carrying this.
    ///
    /// [ADR-0011]: ../../docs/adr/0011-a-permission-is-one-cell-on-the-grid.md
    Grid { rung: Permission, on: LoopId },
}

/// A service principal's token, as the request presented it.
///
/// Nothing resolves one to a principal yet (#57). It is read and carried anyway, because the
/// rule below is about what a request *presents* rather than about who it turns out to be:
/// a token cannot be refused alongside a cookie by a server that never looked for one.
#[derive(Clone)]
// The value itself goes unread until a token resolves to a principal (#57). What this type
// answers today is *was one presented*, which is the whole of the rule above.
pub(crate) struct ServiceToken(#[allow(dead_code)] String);

impl ServiceToken {
    /// Take a token as a caller presented it.
    pub(crate) fn presented(value: String) -> Self {
        Self(value)
    }
}

/// A token is a live credential, and a credential that turns up in a log is spent.
impl std::fmt::Debug for ServiceToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ServiceToken(withheld)")
    }
}

/// Everything a request offers about who is making it, before anything is read.
///
/// Four things and no more. The **sign-in** is what a browser presents and the **service
/// token** is what a script presents, and a request carrying both is refused rather than
/// resolved by precedence (v1 §3).
///
/// The **session** is what a socket that has assumed a role presents. It is a name rather
/// than a claim: it is resolved here, against the state authority, on every message — so a
/// session ended from another tab a moment ago is refused now rather than at the next
/// reconnection. It is not a second credential ([ADR-0041]), because it is only ever read on
/// a channel the sign-in has already authenticated and can only select among that user's own
/// sessions.
///
/// The **acting role** is the one exception: it is handed over as a value by a caller that
/// resolved it some other way, which is the service principal's bound role (#57). A user's
/// acting role is never supplied this way — it is the session's, read here, because a role
/// nobody can be shown to occupy is authority asserted rather than observed.
///
/// [ADR-0041]: ../../docs/adr/0041-a-session-is-resumed-by-name.md
#[derive(Clone, Debug, Default)]
pub(crate) struct Presented {
    sign_in: Option<SignInToken>,
    service_token: Option<ServiceToken>,
    session: Option<SessionId>,
    acting_role: Option<RoleId>,
}

impl Presented {
    /// Whatever the request carried: a sign-in, or nothing at all.
    pub(crate) fn cookie(sign_in: Option<SignInToken>) -> Self {
        Self {
            sign_in,
            service_token: None,
            session: None,
            acting_role: None,
        }
    }

    /// ...and whatever it carried in an `Authorization` header.
    pub(crate) fn and_service_token(self, service_token: Option<ServiceToken>) -> Self {
        Self {
            service_token,
            ..self
        }
    }

    /// ...and the session this socket has assumed a role into, where it has.
    pub(crate) fn in_session(self, session: SessionId) -> Self {
        Self {
            session: Some(session),
            ..self
        }
    }

    /// ...and the role the caller has resolved this principal to be acting through.
    // Nothing but a test supplies one until a service token resolves to a principal (#57):
    // a user's acting role comes from their session and is read here rather than handed in.
    #[allow(dead_code)]
    pub(crate) fn acting_through(self, role: RoleId) -> Self {
        Self {
            acting_role: Some(role),
            ..self
        }
    }

    /// Whether this request presented more than one kind of credential.
    fn mixes_credentials(&self) -> bool {
        self.sign_in.is_some() && self.service_token.is_some()
    }
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
///
/// The state authority is passed for the same reason the store is: two of the six
/// requirements are about a role somebody is **occupying**, which is a live fact and nowhere
/// on disk. Both seams are read here and neither is reached across — this asks each of them
/// a question and composes the answers, which is the only way they ever meet ([ADR-0039]).
///
/// [ADR-0039]: ../../docs/adr/0039-live-state-is-in-process-behind-one-state-authority.md
pub(crate) async fn evaluate(
    requirement: &Requirement,
    presented: Presented,
    store: &Store,
    state: &StateAuthority,
) -> Result<Outcome, StoreError> {
    // **A request carries exactly one credential kind** (v1 §3). Both is refused before the
    // requirement is so much as looked at, and deliberately not resolved by precedence: a
    // confused deputy needs somewhere to be confused, and a precedence order is that place.
    // It is refused on a `Public` operation too, because the rule is about what the request
    // presented rather than about what the operation needs — and nothing legitimate presents
    // two credentials to fetch a stylesheet.
    if presented.mixes_credentials() {
        return Ok(Outcome::Refused);
    }

    match requirement {
        Requirement::Public => Ok(Outcome::Permitted(Caller::Nobody)),

        Requirement::SignedIn => signed_in(presented.sign_in, store, |_| true).await,

        // Gated on the user's flag and **never on a role** (v1 §9), which is why this arm
        // reads the same record `SignedIn` does and asks it one more question. An operator
        // who is also a sysadmin reaches the console without relinquishing, because there is
        // nothing here for a session to satisfy.
        //
        // A locked account is refused as well: locking ends the sign-in, so this is the
        // belt to that braces, and it costs nothing because the record is already read.
        Requirement::SystemAdministration => {
            signed_in(presented.sign_in, store, |user| {
                user.is_system_administrator && !user.is_locked
            })
            .await
        }

        // A role assumed, and still held. Both halves are checked: the sign-in says who is
        // asking, and the state authority says whether the session they named is theirs and
        // is still there. Either one alone would let a socket keep the tier it had when it
        // opened, which is exactly what per-message authorisation exists to stop.
        Requirement::Session => {
            let Some(session) = presented.session.clone() else {
                return Ok(Outcome::Refused);
            };

            signed_in(presented.sign_in, store, |_| true)
                .await
                .map(|outcome| match &outcome {
                    Outcome::Permitted(Caller::User { id, .. })
                        if state.is_held_by(&session, id) =>
                    {
                        outcome
                    }
                    _ => Outcome::Refused,
                })
        }

        Requirement::Grid { rung, on } => carries(presented, store, state, *rung, on).await,

        // A service principal is administered, and none exists yet. It is refused rather
        // than waved through: the default is refusal, everywhere and always.
        Requirement::ServiceToken => Ok(Outcome::Refused),
    }
}

/// Whether the acting principal's role carries `rung` on this loop.
///
/// The whole evaluation is the one lookup at the end of it. Nothing is consulted afterwards
/// and nothing overrides it: an absent cell is `none`, a deliberate `none` is the same
/// `none`, and a loop nobody has ruled on is `none` on every rung whatever its cells hold
/// (v1 §3) — the lookup itself cannot tell the three apart, and neither can this.
///
/// A principal with no acting role is refused rather than checked against something else,
/// because there is nothing else: authority belongs to the role, never to the person.
///
/// For a user the acting role is **the one their session is bound to**, read from the state
/// authority here rather than taken from the caller: reach is never composed across roles
/// and never inherited from eligibility (v1 §1), and a session is the only thing that says
/// which single role somebody is acting through. A bound role handed in by a caller is the
/// service principal's path (#57) and is used as given.
async fn carries(
    presented: Presented,
    store: &Store,
    state: &StateAuthority,
    rung: Permission,
    on: &LoopId,
) -> Result<Outcome, StoreError> {
    let Some(token) = presented.sign_in else {
        return Ok(Outcome::Refused);
    };

    // One transaction for both reads: who is calling, and what their role holds. A second one
    // would let the two answers come from two different moments, on the requirement that runs
    // per socket message.
    let mut transaction = store.begin().await?;
    let read = async {
        // Resolving the principal from a sign-in is *how a user acts*, not what this
        // requirement is about. The other principal it answers for is a service one, which
        // resolves from the token that names it and reaches this same lookup unchanged (#57).
        let Some(user) = whoever_holds(&mut transaction, &token).await? else {
            return Ok(None);
        };

        // The session is resolved **against this user**, so a name belonging to somebody
        // else's session confers nothing — which is what keeps it from being a credential
        // by the back door (ADR-0041).
        let acting_role = match presented.acting_role.clone() {
            handed_over @ Some(_) => handed_over,
            None => presented
                .session
                .as_ref()
                .filter(|session| state.is_held_by(session, &user.id))
                .and_then(|session| state.the_role_of(session)),
        };

        let Some(role) = acting_role else {
            return Ok(None);
        };

        Ok(Some((user, transaction.held_by(&role, on).await?)))
    }
    .await;
    transaction.roll_back().await?;

    Ok(match read? {
        Some((user, held)) if held.carries(rung) => Outcome::Permitted(Caller::User {
            id: user.id,
            sign_in: token,
        }),
        _ => Outcome::Refused,
    })
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
    let resolved = whoever_holds(&mut transaction, &token).await;
    transaction.roll_back().await?;

    Ok(match resolved? {
        Some(user) if holds(&user) => Outcome::Permitted(Caller::User {
            id: user.id,
            sign_in: token,
        }),
        _ => Outcome::Refused,
    })
}

/// The user this sign-in names, as the store has them now.
async fn whoever_holds(
    transaction: &mut crate::configuration::Transaction,
    token: &SignInToken,
) -> Result<Option<crate::configuration::User>, StoreError> {
    let Some(id) = transaction.holder_of(token).await? else {
        return Ok(None);
    };

    transaction.user(&id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::{
        Loops, NewRole, NewUser, PasswordHash, Roles, Users, a_temporary_store,
    };
    use crate::state::Assuming;

    /// A signed-in user and a role they could assume, for the requirements that are about
    /// occupying one.
    async fn a_signed_in_user(store: &Store) -> (SignInToken, UserId, RoleId) {
        a_user_named(store, "flight", "Flight Director").await
    }

    /// Somebody else entirely, on their own sign-in.
    async fn a_second_signed_in_user(store: &Store) -> (SignInToken, UserId, RoleId) {
        a_user_named(store, "gene", "CAPCOM").await
    }

    async fn a_user_named(
        store: &Store,
        username: &str,
        role: &str,
    ) -> (SignInToken, UserId, RoleId) {
        let mut transaction = store.begin().await.expect("a transaction");
        let user = transaction
            .create_user(NewUser {
                username: username.to_owned(),
                password_hash: None,
                is_system_administrator: false,
            })
            .await
            .expect("the user to be created");
        let token = transaction
            .open_sign_in(&user)
            .await
            .expect("the sign-in to open");
        let role = transaction
            .create_role(NewRole {
                name: role.to_owned(),
                max_occupants: Some(1),
            })
            .await
            .expect("the role to be created");
        transaction.commit().await.expect("the deployment to land");

        (token, user, role)
    }

    /// Whoever holds a sign-in, as the store has them.
    async fn holder_of(store: &Store, token: &SignInToken) -> UserId {
        let mut transaction = store.begin().await.expect("a transaction");
        let holder = transaction
            .holder_of(token)
            .await
            .expect("the read to answer");
        transaction.roll_back().await.expect("the read to close");

        holder.expect("a user behind the sign-in")
    }

    fn is_permitted(outcome: &Outcome) -> bool {
        matches!(outcome, Outcome::Permitted(_))
    }

    #[tokio::test]
    async fn a_public_operation_is_permitted_to_nobody_in_particular() {
        let (_directory, store) = a_temporary_store().await;

        let outcome = evaluate(
            &Requirement::Public,
            Presented::default(),
            &store,
            &StateAuthority::empty(),
        )
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

        let outcome = evaluate(
            &Requirement::SignedIn,
            Presented::cookie(Some(token)),
            &store,
            &StateAuthority::empty(),
        )
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

        let outcome = evaluate(
            &Requirement::SignedIn,
            Presented::default(),
            &store,
            &StateAuthority::empty(),
        )
        .await
        .expect("an answer");

        assert!(!is_permitted(&outcome));
    }

    #[tokio::test]
    async fn a_signed_in_operation_is_refused_to_a_token_the_store_does_not_hold() {
        let (_directory, store) = a_temporary_store().await;

        let outcome = evaluate(
            &Requirement::SignedIn,
            Presented::cookie(Some(SignInToken::presented("guessed".to_owned()))),
            &store,
            &StateAuthority::empty(),
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
            &evaluate(
                &Requirement::SignedIn,
                Presented::cookie(Some(token.clone())),
                &store,
                &StateAuthority::empty(),
            )
            .await
            .expect("an answer")
        ));

        let mut transaction = store.begin().await.expect("a transaction");
        transaction
            .end_sign_in(&token)
            .await
            .expect("the sign-in to end");
        transaction.commit().await.expect("the sign-out to land");

        let outcome = evaluate(
            &Requirement::SignedIn,
            Presented::cookie(Some(token)),
            &store,
            &StateAuthority::empty(),
        )
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

        let outcome = evaluate(
            &Requirement::SystemAdministration,
            Presented::cookie(Some(token)),
            &store,
            &StateAuthority::empty(),
        )
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

        let outcome = evaluate(
            &Requirement::SystemAdministration,
            Presented::cookie(Some(token)),
            &store,
            &StateAuthority::empty(),
        )
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
                Presented::cookie(Some(token.clone())),
                &store,
                &StateAuthority::empty(),
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

        let outcome = evaluate(
            &Requirement::SystemAdministration,
            Presented::cookie(Some(token)),
            &store,
            &StateAuthority::empty(),
        )
        .await
        .expect("an answer");
        assert!(!is_permitted(&outcome));
    }
    /// A session is the one thing that satisfies `Session`, and it is refused to a signed-in
    /// user who has assumed nothing — which is what the lobby is.
    #[tokio::test]
    async fn the_session_requirement_is_refused_to_a_user_who_has_assumed_nothing() {
        let (_directory, store) = a_temporary_store().await;
        let (token, _user, _role) = a_signed_in_user(&store).await;

        let outcome = evaluate(
            &Requirement::SignedIn,
            Presented::cookie(Some(token.clone())),
            &store,
            &StateAuthority::empty(),
        )
        .await
        .expect("an answer");
        assert!(is_permitted(&outcome), "the sign-in itself was refused");

        let outcome = evaluate(
            &Requirement::Session,
            Presented::cookie(Some(token)),
            &store,
            &StateAuthority::empty(),
        )
        .await
        .expect("an answer");
        assert!(!is_permitted(&outcome));
    }

    /// ...and it is met once a role is assumed, because the session is read from the state
    /// authority on this call rather than remembered from an earlier one.
    #[tokio::test]
    async fn the_session_requirement_is_met_by_a_role_actually_assumed() {
        let (_directory, store) = a_temporary_store().await;
        let (token, user, role) = a_signed_in_user(&store).await;
        let state = StateAuthority::empty();
        let assumed = state
            .assume(Assuming {
                sign_in: token.clone(),
                occupant: user,
                role,
                limit: Some(1),
            })
            .expect("the seat to be free");

        let outcome = evaluate(
            &Requirement::Session,
            Presented::cookie(Some(token)).in_session(assumed.session),
            &store,
            &state,
        )
        .await
        .expect("an answer");

        assert!(is_permitted(&outcome));
    }

    /// A session belongs to whoever assumed it. Naming somebody else's confers nothing,
    /// which is what keeps the session id from being a credential by the back door
    /// (ADR-0041).
    #[tokio::test]
    async fn a_session_somebody_else_holds_meets_nothing() {
        let (_directory, store) = a_temporary_store().await;
        let (mine, _me, role) = a_signed_in_user(&store).await;
        let (theirs, them, _elsewhere) = a_second_signed_in_user(&store).await;
        let state = StateAuthority::empty();
        let assumed = state
            .assume(Assuming {
                sign_in: theirs,
                occupant: them,
                role,
                limit: Some(1),
            })
            .expect("the seat to be free");

        let outcome = evaluate(
            &Requirement::Session,
            Presented::cookie(Some(mine)).in_session(assumed.session),
            &store,
            &state,
        )
        .await
        .expect("an answer");

        assert!(!is_permitted(&outcome));
    }

    /// A session ended a moment ago takes the tier with it. Nothing is cached, so the
    /// refusal arrives on the next message rather than on the next reconnection.
    #[tokio::test]
    async fn a_session_that_has_ended_meets_nothing() {
        let (_directory, store) = a_temporary_store().await;
        let (token, user, role) = a_signed_in_user(&store).await;
        let state = StateAuthority::empty();
        let assumed = state
            .assume(Assuming {
                sign_in: token.clone(),
                occupant: user,
                role,
                limit: Some(1),
            })
            .expect("the seat to be free");
        state
            .ended_by_its_own_holder(&assumed.session)
            .expect("the session to end");

        let outcome = evaluate(
            &Requirement::Session,
            Presented::cookie(Some(token)).in_session(assumed.session),
            &store,
            &state,
        )
        .await
        .expect("an answer");

        assert!(!is_permitted(&outcome));
    }

    /// **A user's acting role is their session's**, read here rather than handed in. Reach
    /// is never composed across roles and never inherited from eligibility (v1 §1), so a
    /// session is the only thing that says which single role somebody is acting through.
    #[tokio::test]
    async fn the_grid_requirement_reads_the_acting_role_from_the_session() {
        let (_directory, store) = a_temporary_store().await;
        let (token, role, on) = a_role_holding(&store, Permission::Emit).await;
        let user = holder_of(&store, &token).await;
        let state = StateAuthority::empty();
        let assumed = state
            .assume(Assuming {
                sign_in: token.clone(),
                occupant: user,
                role,
                limit: Some(1),
            })
            .expect("the seat to be free");

        let permitted = grid_answer(
            &store,
            &state,
            Presented::cookie(Some(token.clone())).in_session(assumed.session.clone()),
            Permission::Emit,
            &on,
        )
        .await;
        let refused = grid_answer(
            &store,
            &state,
            Presented::cookie(Some(token.clone())).in_session(assumed.session.clone()),
            Permission::Control,
            &on,
        )
        .await;

        assert!(is_permitted(&permitted));
        assert!(!is_permitted(&refused));
    }

    /// ...and a caller who has assumed nothing holds nothing on any loop, whatever the grid
    /// says about roles they are eligible for. Authority belongs to the role, never to the
    /// person.
    #[tokio::test]
    async fn a_user_with_no_session_carries_nothing_on_the_grid() {
        let (_directory, store) = a_temporary_store().await;
        let (token, _role, on) = a_role_holding(&store, Permission::Control).await;

        let outcome = grid_answer(
            &store,
            &StateAuthority::empty(),
            Presented::cookie(Some(token)),
            Permission::Monitor,
            &on,
        )
        .await;

        assert!(!is_permitted(&outcome));
    }

    /// A service principal is administered and none exists yet, so the requirement it
    /// carries is refused rather than waved through: the default is refusal, everywhere and
    /// always.
    #[tokio::test]
    async fn the_service_token_requirement_is_refused_for_want_of_a_principal() {
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

        let outcome = evaluate(
            &Requirement::ServiceToken,
            Presented::cookie(Some(token)),
            &store,
            &StateAuthority::empty(),
        )
        .await
        .expect("an answer");

        assert!(!is_permitted(&outcome));
    }

    /// **A request carries exactly one credential kind** (v1 §3). Presenting a cookie and a
    /// token together is refused rather than resolved by precedence, whatever the operation
    /// would have said about either one on its own.
    #[tokio::test]
    async fn a_request_presenting_both_a_cookie_and_a_token_is_refused() {
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

        for requirement in [
            Requirement::Public,
            Requirement::SignedIn,
            Requirement::SystemAdministration,
        ] {
            let outcome = evaluate(
                &requirement,
                Presented::cookie(Some(token.clone()))
                    .and_service_token(Some(ServiceToken::presented("a-service-token".to_owned()))),
                &store,
                &StateAuthority::empty(),
            )
            .await
            .expect("an answer");

            assert!(
                !is_permitted(&outcome),
                "{requirement:?} resolved a request carrying two credentials"
            );
        }
    }

    /// A token is a live credential, and one that turns up in a log is spent.
    #[test]
    fn a_service_token_does_not_print_itself() {
        let token = ServiceToken::presented("a-service-token".to_owned());

        assert_eq!(format!("{token:?}"), "ServiceToken(withheld)");
    }

    /// A signed-in user, and a loop somebody has ruled on with this role's cell set to
    /// `held`. The loop is ruled on because an unreviewed one answers `none` on every rung
    /// whatever its cells say, which would make these pass for the wrong reason.
    async fn a_role_holding(store: &Store, held: Permission) -> (SignInToken, RoleId, LoopId) {
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
        let role = transaction
            .create_role(NewRole {
                name: "Flight Director".to_owned(),
                max_occupants: Some(1),
            })
            .await
            .expect("the role to be created");
        let on = transaction
            .create_loop("FLIGHT")
            .await
            .expect("the loop to be created");
        transaction
            .dismiss_unreviewed(&on)
            .await
            .expect("the loop to be ruled on");
        transaction
            .set_cell(&role, &on, held)
            .await
            .expect("the cell to be set");
        transaction.commit().await.expect("the deployment to land");

        (token, role, on)
    }

    async fn grid_answer(
        store: &Store,
        state: &StateAuthority,
        presented: Presented,
        rung: Permission,
        on: &LoopId,
    ) -> Outcome {
        evaluate(
            &Requirement::Grid {
                rung,
                on: on.clone(),
            },
            presented,
            store,
            state,
        )
        .await
        .expect("an answer")
    }

    /// Each rung carries everything below it, and nothing above it.
    #[tokio::test]
    async fn the_grid_requirement_answers_the_cell_the_acting_role_holds() {
        let (_directory, store) = a_temporary_store().await;
        let (token, role, on) = a_role_holding(&store, Permission::Emit).await;
        let presented = Presented::cookie(Some(token)).acting_through(role);

        for (rung, expected) in [
            (Permission::None, true),
            (Permission::Monitor, true),
            (Permission::Emit, true),
            (Permission::Control, false),
        ] {
            let outcome = grid_answer(
                &store,
                &StateAuthority::empty(),
                presented.clone(),
                rung,
                &on,
            )
            .await;

            assert_eq!(
                is_permitted(&outcome),
                expected,
                "a role holding emit answered the wrong thing about {rung:?}"
            );
        }
    }

    /// The evaluator enforces an unreviewed loop's cells as `none`, **with no exception**,
    /// and cannot tell that from a deliberate `none` (v1 §3). Both halves are one test,
    /// because the property is that the two are the same answer.
    #[tokio::test]
    async fn an_unreviewed_loop_is_none_and_is_indistinguishable_from_a_deliberate_none() {
        let (_directory, store) = a_temporary_store().await;
        let (token, role, on) = a_role_holding(&store, Permission::Control).await;
        let presented = Presented::cookie(Some(token)).acting_through(role.clone());
        assert!(
            is_permitted(
                &grid_answer(
                    &store,
                    &StateAuthority::empty(),
                    presented.clone(),
                    Permission::Control,
                    &on
                )
                .await
            ),
            "a ruled-on loop refused the control its cell holds"
        );

        let mut transaction = store.begin().await.expect("a transaction");
        let unreviewed = transaction
            .create_loop("GNC")
            .await
            .expect("the loop to be created");
        transaction
            .set_cell(&role, &unreviewed, Permission::Control)
            .await
            .expect("the cell to be set");
        let deliberate = transaction
            .create_loop("THERMAL")
            .await
            .expect("the loop to be created");
        transaction
            .dismiss_unreviewed(&deliberate)
            .await
            .expect("the loop to be ruled on");
        transaction
            .set_cell(&role, &deliberate, Permission::None)
            .await
            .expect("the deliberate none to be recorded");
        transaction.commit().await.expect("the writes to land");

        for rung in [Permission::Monitor, Permission::Emit, Permission::Control] {
            assert!(
                !is_permitted(
                    &grid_answer(
                        &store,
                        &StateAuthority::empty(),
                        presented.clone(),
                        rung,
                        &unreviewed
                    )
                    .await
                ),
                "an unreviewed loop conferred {rung:?} on the strength of a cell somebody set"
            );
            assert!(
                !is_permitted(
                    &grid_answer(
                        &store,
                        &StateAuthority::empty(),
                        presented.clone(),
                        rung,
                        &deliberate
                    )
                    .await
                ),
                "a deliberate none conferred {rung:?}"
            );
        }
    }

    /// Authority belongs to the role, so a principal acting through none has none — and a
    /// role nobody is acting through is not consulted either.
    #[tokio::test]
    async fn a_principal_acting_through_no_role_holds_nothing() {
        let (_directory, store) = a_temporary_store().await;
        let (token, _role, on) = a_role_holding(&store, Permission::Control).await;

        let outcome = grid_answer(
            &store,
            &StateAuthority::empty(),
            Presented::cookie(Some(token)),
            Permission::Monitor,
            &on,
        )
        .await;

        assert!(!is_permitted(&outcome));
    }

    /// There is no per-user layer anywhere in the evaluator (ADR-0011). The
    /// system-administration flag configures the grid and confers nothing on it, so an
    /// administrator acting through a role with no cell is refused exactly as anybody else
    /// acting through it is.
    #[tokio::test]
    async fn holding_the_system_administration_flag_confers_no_reach() {
        let (_directory, store) = a_temporary_store().await;
        let (_token, role, on) = a_role_holding(&store, Permission::None).await;
        let mut transaction = store.begin().await.expect("a transaction");
        let administrator = transaction
            .create_user(NewUser {
                username: "root".to_owned(),
                password_hash: None,
                is_system_administrator: true,
            })
            .await
            .expect("an administrator");
        let token = transaction
            .open_sign_in(&administrator)
            .await
            .expect("the sign-in to open");
        transaction.commit().await.expect("the sign-in to land");

        let outcome = grid_answer(
            &store,
            &StateAuthority::empty(),
            Presented::cookie(Some(token)).acting_through(role),
            Permission::Monitor,
            &on,
        )
        .await;

        assert!(
            !is_permitted(&outcome),
            "the system-administration flag was read as reach"
        );
    }

    /// Reach is never composed across the roles a person may hold: the acting role is the
    /// whole of the answer, and another role's row is not consulted (v1 §1).
    #[tokio::test]
    async fn reach_is_the_acting_roles_row_and_never_another_roles() {
        let (_directory, store) = a_temporary_store().await;
        let (token, _reaching, on) = a_role_holding(&store, Permission::Control).await;
        let mut transaction = store.begin().await.expect("a transaction");
        let observer = transaction
            .create_role(NewRole {
                name: "Observer II".to_owned(),
                max_occupants: None,
            })
            .await
            .expect("a second role");
        transaction.commit().await.expect("the role to land");

        let outcome = grid_answer(
            &store,
            &StateAuthority::empty(),
            Presented::cookie(Some(token)).acting_through(observer),
            Permission::Monitor,
            &on,
        )
        .await;

        assert!(
            !is_permitted(&outcome),
            "a role with no cell was answered from another role's row"
        );
    }
}
