//! Local passwords: the first implementation of the identity seam, not a bypass around it
//! ([ADR-0024]).
//!
//! Argon2id, a twelve-character floor, no forced rotation and no complexity rules
//! ([ADR-0025]). Rotation and character-class requirements measurably produce worse
//! passwords, and a stricter local policy than the customer's eventual identity provider is
//! work thrown away twice.
//!
//! Nothing outside Identity ever sees what is inside a stored hash. Configuration keeps one
//! and hands it back; this is the only code that knows what it means.
//!
//! [ADR-0024]: ../../../docs/adr/0024-identity-is-a-replaceable-front-door.md
//! [ADR-0025]: ../../../docs/adr/0025-credentials-are-administered-because-there-is-no-email.md

use std::sync::OnceLock;

use argon2::password_hash::phc;
use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use async_trait::async_trait;

use super::{FrontDoor, Presented};
use crate::configuration::{PasswordHash, StoreError, Transaction, UserId, Users};

/// The shortest password VoxLoop will store. Length is the only rule there is.
const FLOOR: usize = 12;

/// Why a password was not accepted. There are two reasons and neither is a matter of taste.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PasswordRefused {
    #[error("a password must be at least {FLOOR} characters long")]
    TooShort,

    #[error("the password could not be stored")]
    Unusable,
}

/// The local password adapter: usernames and Argon2id hashes, behind the identity seam.
#[derive(Clone, Copy, Default)]
pub(crate) struct LocalPasswords;

impl LocalPasswords {
    /// Hash a password for storage, refusing one under the floor.
    pub(super) fn hash(self, password: &str) -> Result<PasswordHash, PasswordRefused> {
        // Characters rather than bytes: a twelve-character passphrase is twelve characters
        // whatever alphabet it is written in.
        if password.chars().count() < FLOOR {
            return Err(PasswordRefused::TooShort);
        }

        let hashed: phc::PasswordHash = Argon2::default()
            .hash_password(password.as_bytes())
            .map_err(|_| PasswordRefused::Unusable)?;

        Ok(PasswordHash::already_hashed(hashed.to_string()))
    }

    /// Whether this is the password that user holds.
    ///
    /// This is local password administration rather than the front door: it starts from a
    /// user id the caller has already been given, so it proves nothing about who anybody is
    /// and resolves nobody. Re-presenting the current password to change it is what it is
    /// for, and a user with no password yet holds none to re-present.
    pub(super) async fn confirms(
        self,
        transaction: &mut Transaction,
        user: &UserId,
        password: &str,
    ) -> Result<bool, StoreError> {
        let Some(stored) = transaction.password_held_by(user).await? else {
            self.spend_the_time_a_check_would_have(password);
            return Ok(false);
        };

        Ok(self.matches(password, &stored))
    }

    /// Check a password against a stored hash.
    fn matches(self, password: &str, stored: &PasswordHash) -> bool {
        let Ok(parsed) = phc::PasswordHash::new(stored.as_str()) else {
            return false;
        };

        Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok()
    }

    /// Spend what a real check would have spent, against a hash of nothing.
    ///
    /// Without this, a username nobody holds answers faster than one somebody does, and the
    /// sign-in route becomes a way to enumerate the people on the deployment.
    fn spend_the_time_a_check_would_have(self, password: &str) {
        static NOBODY: OnceLock<PasswordHash> = OnceLock::new();

        let decoy = NOBODY.get_or_init(|| {
            self.hash("a password nobody holds")
                .unwrap_or_else(|_| PasswordHash::already_hashed(String::new()))
        });

        let _ = self.matches(password, decoy);
    }
}

#[async_trait]
impl FrontDoor for LocalPasswords {
    async fn resolve(
        &self,
        transaction: &mut Transaction,
        presented: &Presented,
    ) -> Result<Option<UserId>, StoreError> {
        let Presented::Password { username, password } = presented;

        let Some(stored) = transaction.stored_password(username).await? else {
            self.spend_the_time_a_check_would_have(password);
            return Ok(None);
        };

        if self.matches(password, &stored.hash) {
            Ok(Some(stored.user))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::{NewUser, a_temporary_store};

    async fn a_user_with(transaction: &mut Transaction, username: &str, password: &str) -> UserId {
        transaction
            .create_user(NewUser {
                username: username.to_owned(),
                password_hash: Some(LocalPasswords.hash(password).expect("the password to hash")),
                is_system_administrator: false,
            })
            .await
            .expect("the user to be created")
    }

    fn presented(username: &str, password: &str) -> Presented {
        Presented::Password {
            username: username.to_owned(),
            password: password.to_owned(),
        }
    }

    #[test]
    fn refuses_a_password_under_the_floor_and_accepts_one_at_it() {
        assert!(matches!(
            LocalPasswords.hash(&"a".repeat(FLOOR - 1)),
            Err(PasswordRefused::TooShort)
        ));
        assert!(LocalPasswords.hash(&"a".repeat(FLOOR)).is_ok());
    }

    #[test]
    fn counts_the_floor_in_characters_rather_than_bytes() {
        // Twelve characters, well over twelve bytes.
        assert!(LocalPasswords.hash("ααααααααααββ").is_ok());
        assert!(matches!(
            LocalPasswords.hash("ααααααααααβ"),
            Err(PasswordRefused::TooShort)
        ));
    }

    #[test]
    fn imposes_no_rule_but_length() {
        for password in [
            "aaaaaaaaaaaa",
            "correct horse battery staple",
            "            ",
            "🛰🛰🛰🛰🛰🛰🛰🛰🛰🛰🛰🛰",
        ] {
            assert!(
                LocalPasswords.hash(password).is_ok(),
                "{password:?} was refused"
            );
        }
    }

    #[test]
    fn stores_a_hash_rather_than_the_password() {
        let stored = LocalPasswords
            .hash("a long enough password")
            .expect("the password to hash");

        assert!(!stored.as_str().contains("a long enough password"));
        assert!(stored.as_str().starts_with("$argon2id$"));
    }

    #[test]
    fn hashes_the_same_password_differently_every_time() {
        let once = LocalPasswords
            .hash("a long enough password")
            .expect("a hash");
        let twice = LocalPasswords
            .hash("a long enough password")
            .expect("a hash");

        assert_ne!(once.as_str(), twice.as_str(), "the hash is unsalted");
        assert!(LocalPasswords.matches("a long enough password", &once));
        assert!(LocalPasswords.matches("a long enough password", &twice));
    }

    #[tokio::test]
    async fn resolves_the_right_password_to_the_user_who_holds_it() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = a_user_with(&mut transaction, "flight", "a long enough password").await;

        let resolved = LocalPasswords
            .resolve(
                &mut transaction,
                &presented("flight", "a long enough password"),
            )
            .await
            .expect("the check to answer");

        assert_eq!(resolved, Some(user));
    }

    #[tokio::test]
    async fn resolves_nobody_from_a_wrong_password_an_unknown_name_or_no_password_at_all() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        a_user_with(&mut transaction, "flight", "a long enough password").await;
        transaction
            .create_user(NewUser {
                username: "enrolling".to_owned(),
                password_hash: None,
                is_system_administrator: false,
            })
            .await
            .expect("a user with no password yet");

        for attempt in [
            presented("flight", "the wrong password"),
            presented("nobody", "a long enough password"),
            presented("enrolling", "a long enough password"),
        ] {
            assert_eq!(
                LocalPasswords
                    .resolve(&mut transaction, &attempt)
                    .await
                    .expect("the check to answer"),
                None,
                "{attempt:?} resolved to somebody"
            );
        }
    }

    #[tokio::test]
    async fn resolves_a_name_however_it_was_capitalised() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = a_user_with(&mut transaction, "flight", "a long enough password").await;

        let resolved = LocalPasswords
            .resolve(
                &mut transaction,
                &presented("Flight", "a long enough password"),
            )
            .await
            .expect("the check to answer");

        assert_eq!(resolved, Some(user));
    }

    #[tokio::test]
    async fn confirms_the_password_a_user_holds_and_nothing_else() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = a_user_with(&mut transaction, "flight", "a long enough password").await;

        assert!(
            LocalPasswords
                .confirms(&mut transaction, &user, "a long enough password")
                .await
                .expect("the check to answer")
        );
        assert!(
            !LocalPasswords
                .confirms(&mut transaction, &user, "the wrong password")
                .await
                .expect("the check to answer")
        );
    }

    /// A user awaiting enrolment holds nothing to re-present, so there is nothing that
    /// confirms — least of all an empty string.
    #[tokio::test]
    async fn confirms_nothing_for_a_user_who_holds_no_password_yet() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = transaction
            .create_user(NewUser {
                username: "enrolling".to_owned(),
                password_hash: None,
                is_system_administrator: false,
            })
            .await
            .expect("a user with no password yet");

        for attempt in ["", "a long enough password"] {
            assert!(
                !LocalPasswords
                    .confirms(&mut transaction, &user, attempt)
                    .await
                    .expect("the check to answer"),
                "{attempt:?} confirmed against an account with no password"
            );
        }
    }
}
