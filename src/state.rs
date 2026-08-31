//! State authority — every live fact about the running system, and the only writer of any
//! of them.
//!
//! Sessions, occupancy, subscriptions, arms, key state, connection state and loop health all
//! live here, in plain structures owned by this process. There is no second store and there
//! is no Redis ([ADR-0039]): a restart genuinely ends every session, because the media
//! plane cannot survive one at any price and occupancy restored without an audio path would
//! be exactly the lie the product exists to avoid. Users stay **signed in** across a restart
//! — that is durable and lives in Configuration — and must assume their role again.
//!
//! Being the single writer is the point rather than a side effect. Presence documents are
//! projections this module computes rather than records it keeps, which is what lets their
//! versions be monotonic and what they show be simultaneously true ([ADR-0019]).
//!
//! **What is here today is what #36 needs and no more.** The lobby asks who occupies each
//! role, and the sign-in clock asks which sign-ins hold a session; both are questions about
//! occupancy, and occupancy is created by assuming a role — which is #37's operation and
//! the one writer this module is waiting for.
//!
//! [ADR-0019]: ../../docs/adr/0019-presence-is-one-versioned-document-scoped-to-reach.md
//! [ADR-0039]: ../../docs/adr/0039-live-state-is-in-process-behind-one-state-authority.md

use std::sync::Mutex;

use crate::configuration::{RoleId, SignInToken, UserId};

/// A user's single live connection to the voice loops, bound to exactly one role.
///
/// It carries the sign-in it was assumed from, because the two acts have two lifetimes and
/// the outer one has a clock the inner one stops ([ADR-0023]): a sign-in standing in the
/// lobby ends after 24 hours of nothing, and a sign-in holding one of these does not.
///
/// [ADR-0023]: ../../docs/adr/0023-sign-in-is-to-the-application-and-a-role-is-assumed.md
struct Session {
    sign_in: SignInToken,
    occupant: UserId,
    role: RoleId,
}

/// Everything live, behind one lock so there is one writer.
#[derive(Default)]
struct Live {
    /// One per occupied seat. A user has at most one, though they may be signed in on
    /// several machines (v1 §2).
    sessions: Vec<Session>,
}

/// The single holder of live state, and the only thing that may read or write it.
///
/// It is shared rather than owned: Transport asks it what to render and the sign-in clock
/// asks it who is on shift, and neither reaches the structures behind it.
#[derive(Default)]
pub(crate) struct StateAuthority {
    live: Mutex<Live>,
}

impl StateAuthority {
    /// A running system with nobody on it, which is what a restart leaves.
    pub(crate) fn empty() -> Self {
        Self::default()
    }

    /// Who occupies this role, now.
    ///
    /// Occupancy means a role somebody has assumed and not relinquished — never somebody
    /// merely signed in, and never somebody eligible ([ADR-0005]). An empty answer is
    /// *nobody is in that seat*, which is the answer the lobby exists to give.
    ///
    /// [ADR-0005]: ../../docs/adr/0005-occupancy-means-listening-not-signed-in.md
    pub(crate) fn occupants_of(&self, role: &RoleId) -> Vec<UserId> {
        self.read(|live| {
            live.sessions
                .iter()
                .filter(|session| &session.role == role)
                .map(|session| session.occupant.clone())
                .collect()
        })
    }

    /// The sign-ins that hold a session.
    ///
    /// They are what the 24-hour window spares, because that clock **runs only in the
    /// lobby** ([ADR-0023]). The answer is handed to Configuration as a value, which is the
    /// only way the live side and the durable side ever meet ([ADR-0039]).
    ///
    /// [ADR-0023]: ../../docs/adr/0023-sign-in-is-to-the-application-and-a-role-is-assumed.md
    /// [ADR-0039]: ../../docs/adr/0039-live-state-is-in-process-behind-one-state-authority.md
    pub(crate) fn sign_ins_holding_a_session(&self) -> Vec<SignInToken> {
        self.read(|live| {
            live.sessions
                .iter()
                .map(|session| session.sign_in.clone())
                .collect()
        })
    }

    /// Read live state under the lock.
    ///
    /// A poisoned lock is a panic somewhere else in this module, and the honest answer to
    /// *what is live* after one is *nothing I can vouch for*. Recovering the structures and
    /// carrying on would be reading state a panic left half-written, so this takes the
    /// answer that is true either way.
    fn read<T>(&self, of: impl FnOnce(&Live) -> T) -> T {
        match self.live.lock() {
            Ok(live) => of(&live),
            Err(poisoned) => of(&poisoned.into_inner()),
        }
    }

    /// Put a session in, standing in for the assume that creates one (#37).
    ///
    /// Occupancy has exactly one origin — the explicit act of assuming a role — and building
    /// half of that act here to have something to test against would be building the wrong
    /// half. This is the seam the reads above are exercised through until the act exists.
    #[cfg(test)]
    pub(crate) fn a_session_is_held(
        &self,
        sign_in: &SignInToken,
        occupant: &UserId,
        role: &RoleId,
    ) {
        let mut live = self
            .live
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        live.sessions.push(Session {
            sign_in: sign_in.clone(),
            occupant: occupant.clone(),
            role: role.clone(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::{NewRole, NewUser, Roles, SignIns, Store, Users, a_temporary_store};

    /// A user, a role, and the sign-in the role would be assumed from.
    async fn a_seat(store: &Store, username: &str, role: &str) -> (SignInToken, UserId, RoleId) {
        let mut transaction = store.begin().await.expect("a transaction");
        let user = transaction
            .create_user(NewUser {
                username: username.to_owned(),
                password_hash: None,
                is_system_administrator: false,
            })
            .await
            .expect("the user to be created");
        let sign_in = transaction
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

        (sign_in, user, role)
    }

    #[tokio::test]
    async fn a_role_nobody_has_assumed_has_no_occupants() {
        let (_directory, store) = a_temporary_store().await;
        let (_sign_in, _user, role) = a_seat(&store, "flight", "Flight Director").await;

        assert!(StateAuthority::empty().occupants_of(&role).is_empty());
    }

    /// Occupancy is per role, so the answer names whoever is in that seat and nobody in any
    /// other one.
    #[tokio::test]
    async fn a_role_answers_whoever_occupies_it_and_nobody_else() {
        let (_directory, store) = a_temporary_store().await;
        let (sign_in, user, role) = a_seat(&store, "flight", "Flight Director").await;
        let (elsewhere, capcom, another) = a_seat(&store, "capcom", "CAPCOM").await;
        let live = StateAuthority::empty();

        live.a_session_is_held(&sign_in, &user, &role);
        live.a_session_is_held(&elsewhere, &capcom, &another);

        assert_eq!(live.occupants_of(&role), vec![user]);
        assert_eq!(live.occupants_of(&another), vec![capcom]);
    }

    /// The clock runs only in the lobby, and this is the half of that rule the live side
    /// answers: which sign-ins are not standing in it.
    #[tokio::test]
    async fn the_sign_ins_holding_a_session_are_the_ones_that_assumed_a_role() {
        let (_directory, store) = a_temporary_store().await;
        let (sign_in, user, role) = a_seat(&store, "flight", "Flight Director").await;
        let (in_the_lobby, _capcom, _another) = a_seat(&store, "capcom", "CAPCOM").await;
        let live = StateAuthority::empty();
        assert!(live.sign_ins_holding_a_session().is_empty());

        live.a_session_is_held(&sign_in, &user, &role);

        let holding = live.sign_ins_holding_a_session();
        assert_eq!(holding.len(), 1);
        assert_eq!(holding[0].as_str(), sign_in.as_str());
        assert_ne!(holding[0].as_str(), in_the_lobby.as_str());
    }
}
