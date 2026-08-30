//! Identity — resolves a credential to an internal user id, and nothing else.
//!
//! The front door is replaceable and authorisation never leaves VoxLoop ([ADR-0024]). The
//! adapter's **entire output is a resolved internal user id**: nothing downstream — lobby,
//! eligibility, the grid, sessions, audit — ever learns how the principal authenticated,
//! because there is nothing else here for a replacement to have to match.
//!
//! Local passwords are the first implementation of that seam rather than a bypass around it,
//! which is why this module exists in v1 with one adapter behind it. An OIDC or SAML
//! implementation is a second [`FrontDoor`] and a line in [`Identity::local_passwords`]'s
//! place; nothing above it changes.
//!
//! **The seam carries identity only** — no groups, no claims-to-roles mapping. Permissions
//! stay in the grid.
//!
//! [ADR-0024]: ../../../docs/adr/0024-identity-is-a-replaceable-front-door.md

mod bootstrap;
mod passwords;

use std::sync::Arc;

use async_trait::async_trait;

use crate::configuration::{PasswordHash, StoreError, Transaction, UserId};
pub(crate) use bootstrap::{Bootstrap, Redemption};
pub(crate) use passwords::PasswordRefused;

/// A credential as somebody presented it.
///
/// One variant today. An identity provider's assertion is the second, and adding it is what
/// this enum is for.
pub(crate) enum Presented {
    Password { username: String, password: String },
}

/// A presented password is a live credential, and one that turns up in a log is spent.
impl std::fmt::Debug for Presented {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self::Password { username, .. } = self;
        write!(
            formatter,
            "Password {{ username: {username:?}, password: withheld }}"
        )
    }
}

/// The front door itself: whatever turns a presented credential into a user id.
///
/// The interface is one method answering one value, and that is deliberate. Anything else it
/// returned would be something a replacement had to match, and the point of this seam is
/// that there is nothing to match.
#[async_trait]
trait FrontDoor: Send + Sync {
    /// Resolve a presented credential to the user it names, or to nobody.
    ///
    /// Nobody is `Ok(None)` rather than an error: a wrong password is an ordinary answer,
    /// and the caller learns which user was named only when the answer is somebody.
    async fn resolve(
        &self,
        transaction: &mut Transaction,
        presented: &Presented,
    ) -> Result<Option<UserId>, StoreError>;
}

/// VoxLoop's identity, with whichever front door this deployment was built with.
///
/// The two fields are not one field twice. `front_door` is the replaceable part, and an
/// identity provider takes its place without anything above noticing. `passwords` is local
/// password administration, which stays whatever the front door becomes, because a local
/// break-glass administrator is permanent ([ADR-0024]). Today one type happens to serve
/// both; the day an identity provider arrives, only the first changes.
///
/// [ADR-0024]: ../../../docs/adr/0024-identity-is-a-replaceable-front-door.md
#[derive(Clone)]
pub(crate) struct Identity {
    front_door: Arc<dyn FrontDoor>,
    passwords: passwords::LocalPasswords,
}

impl Identity {
    /// Local accounts: usernames and Argon2id password hashes in VoxLoop's own store.
    pub(crate) fn local_passwords() -> Self {
        Self {
            front_door: Arc::new(passwords::LocalPasswords),
            passwords: passwords::LocalPasswords,
        }
    }

    /// Resolve a presented credential to the user it names, or to nobody.
    pub(crate) async fn resolve(
        &self,
        transaction: &mut Transaction,
        presented: &Presented,
    ) -> Result<Option<UserId>, StoreError> {
        self.front_door.resolve(transaction, presented).await
    }

    /// Hash a password for storage, refusing one under the floor.
    ///
    /// This is local password administration rather than the front door, and it stays here
    /// whatever the front door becomes: a local break-glass administrator is permanent, and
    /// survives SSO adoption ([ADR-0024]).
    ///
    /// [ADR-0024]: ../../../docs/adr/0024-identity-is-a-replaceable-front-door.md
    pub(crate) fn hash_password(&self, password: &str) -> Result<PasswordHash, PasswordRefused> {
        self.passwords.hash(password)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::{NewUser, Users, a_temporary_store};

    #[tokio::test]
    async fn the_whole_of_what_the_front_door_answers_is_a_user_id() {
        let (_directory, store) = a_temporary_store().await;
        let identity = Identity::local_passwords();
        let mut transaction = store.begin().await.expect("a transaction");
        let user = transaction
            .create_user(NewUser {
                username: "flight".to_owned(),
                password_hash: Some(
                    identity
                        .hash_password("a long enough password")
                        .expect("the password to hash"),
                ),
                is_system_administrator: true,
            })
            .await
            .expect("the user to be created");

        let resolved: Option<UserId> = identity
            .resolve(
                &mut transaction,
                &Presented::Password {
                    username: "flight".to_owned(),
                    password: "a long enough password".to_owned(),
                },
            )
            .await
            .expect("the check to answer");

        // A user id and nothing else: not the flag, not the username, not how it was checked.
        assert_eq!(resolved, Some(user));
    }

    #[test]
    fn a_presented_password_is_withheld_from_anything_that_prints_it() {
        let presented = Presented::Password {
            username: "flight".to_owned(),
            password: "a long enough password".to_owned(),
        };

        assert!(!format!("{presented:?}").contains("a long enough password"));
    }
}
