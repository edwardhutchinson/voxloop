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

/// What an operation demands of whoever calls it.
///
/// ADR-0054 fixes six requirements and no seventh. Five are a function of the caller alone.
/// The sixth, [`Requirement::Grid`], is a function of the operation's *arguments* as well —
/// it names a loop the caller supplies — so every operation carrying it is a
/// signalling-channel message rather than an HTTP route (`docs/spec/api-surface.md`), built
/// per message rather than registered once.
// Two of the six name something no principal can hold yet: a role (#37) or a service token
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

/// Everything a request offers about who is making it, before anything is read from the
/// store.
///
/// Two things and no more. The **sign-in** is what a browser presents. The **acting role** is
/// resolved by whoever calls this rather than read here — it is a session's assumed role
/// (#37) or a service token's bound role (#57), and both are live facts this module does not
/// reach for. Nothing supplies one yet, so `Grid` is refused today for want of a principal to
/// check, which is the same default everything else here has.
#[derive(Clone, Debug, Default)]
pub(crate) struct Presented {
    sign_in: Option<SignInToken>,
    acting_role: Option<RoleId>,
}

impl Presented {
    /// Whatever the request carried: a sign-in, or nothing at all.
    pub(crate) fn cookie(sign_in: Option<SignInToken>) -> Self {
        Self {
            sign_in,
            acting_role: None,
        }
    }

    /// ...and the role the caller has resolved this principal to be acting through.
    // Nothing but a test supplies one until a role can be assumed (#37). It is here rather
    // than with that ticket because the requirement it feeds is fixed by ADR-0054, and a
    // requirement nothing can satisfy is a requirement nothing has tested.
    #[allow(dead_code)]
    pub(crate) fn acting_through(self, role: RoleId) -> Self {
        Self {
            acting_role: Some(role),
            ..self
        }
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
pub(crate) async fn evaluate(
    requirement: &Requirement,
    presented: Presented,
    store: &Store,
) -> Result<Outcome, StoreError> {
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

        Requirement::Grid { rung, on } => carries(presented, store, *rung, on).await,

        // A role is assumed over the signalling channel and a service principal is
        // administered, and neither exists yet. They are refused rather than waved through:
        // the default is refusal, everywhere and always.
        Requirement::Session | Requirement::ServiceToken => Ok(Outcome::Refused),
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
async fn carries(
    presented: Presented,
    store: &Store,
    rung: Permission,
    on: &LoopId,
) -> Result<Outcome, StoreError> {
    let (Some(role), Some(token)) = (presented.acting_role, presented.sign_in) else {
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

    fn is_permitted(outcome: &Outcome) -> bool {
        matches!(outcome, Outcome::Permitted(_))
    }

    #[tokio::test]
    async fn a_public_operation_is_permitted_to_nobody_in_particular() {
        let (_directory, store) = a_temporary_store().await;

        let outcome = evaluate(&Requirement::Public, Presented::default(), &store)
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

        let outcome = evaluate(&Requirement::SignedIn, Presented::default(), &store)
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
                &store
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

        let outcome = evaluate(
            &Requirement::SystemAdministration,
            Presented::cookie(Some(token)),
            &store,
        )
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
            let outcome = evaluate(&requirement, Presented::cookie(Some(token.clone())), &store)
                .await
                .expect("an answer");

            assert!(
                !is_permitted(&outcome),
                "expected {requirement:?} to be refused"
            );
        }
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
            let outcome = grid_answer(&store, presented.clone(), rung, &on).await;

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
            is_permitted(&grid_answer(&store, presented.clone(), Permission::Control, &on).await),
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
                !is_permitted(&grid_answer(&store, presented.clone(), rung, &unreviewed).await),
                "an unreviewed loop conferred {rung:?} on the strength of a cell somebody set"
            );
            assert!(
                !is_permitted(&grid_answer(&store, presented.clone(), rung, &deliberate).await),
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
