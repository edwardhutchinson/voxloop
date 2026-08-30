//! Sign-ins: what a signed-in browser is holding, and who it belongs to.
//!
//! A sign-in is durable. VoxLoop's lifetime table (v1 §2) has occupancy ending at a server
//! restart and the **sign-in surviving** it, which is what puts sign-ins in the store rather
//! than with the live state the state authority holds and discards.
//!
//! What is stored is a fingerprint of the token, never the token: the store is one file a
//! deployment is obliged to back up, and a backup should not be a drawer full of usable
//! sign-ins.

use async_trait::async_trait;
use sqlx::Row;

use super::store::{StoreError, Transaction, now, unavailable};
use super::users::UserId;
use crate::secrets;

/// The opaque value a signed-in browser holds, and the whole of what it holds.
///
/// It carries no claims — not the username, not the system-administration flag, not a role.
/// That is what makes revocation immediate: everything about the caller is read from the
/// store per request, so nothing can be true of a cookie that is no longer true of the user
/// (v1 §3).
#[derive(Clone)]
pub(crate) struct SignInToken(String);

impl SignInToken {
    /// Take a token as a browser presented it.
    pub(crate) fn presented(value: String) -> Self {
        Self(value)
    }

    /// The value to hand the browser. Every other use of this is a mistake.
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// A token is a live credential, and a credential that turns up in a log is spent.
impl std::fmt::Debug for SignInToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SignInToken(withheld)")
    }
}

/// The sign-ins a user holds, as domain operations rather than queries.
#[async_trait]
pub(crate) trait SignIns {
    /// Sign a user in, and answer with the token that says so.
    ///
    /// A user may hold several — one per machine (v1 §2) — and each is ended on its own.
    async fn open_sign_in(&mut self, user: &UserId) -> Result<SignInToken, StoreError>;

    /// Who this token signs in, if it still signs anybody in.
    async fn holder_of(&mut self, token: &SignInToken) -> Result<Option<UserId>, StoreError>;

    /// End this sign-in, answering with whom it belonged to so the act can be recorded.
    async fn end_sign_in(&mut self, token: &SignInToken) -> Result<Option<UserId>, StoreError>;

    /// End every sign-in this user holds, on every machine.
    ///
    /// Locking an account and forcing a password reset both end the sign-in rather than
    /// waiting for it to expire (v1 §2's lifetime table), and a user may hold one per
    /// machine — so signing them out means all of them or none.
    async fn end_every_sign_in(&mut self, user: &UserId) -> Result<(), StoreError>;
}

#[async_trait]
impl SignIns for Transaction {
    async fn open_sign_in(&mut self, user: &UserId) -> Result<SignInToken, StoreError> {
        let token = SignInToken(secrets::unguessable());

        sqlx::query("INSERT INTO sign_ins (fingerprint, user_id, started_at) VALUES (?, ?, ?)")
            .bind(secrets::fingerprint(&token.0))
            .bind(user.as_str())
            .bind(now())
            .execute(self.connection())
            .await
            .map_err(unavailable)?;

        Ok(token)
    }

    async fn holder_of(&mut self, token: &SignInToken) -> Result<Option<UserId>, StoreError> {
        let found = sqlx::query("SELECT user_id FROM sign_ins WHERE fingerprint = ?")
            .bind(secrets::fingerprint(&token.0))
            .fetch_optional(self.connection())
            .await
            .map_err(unavailable)?;

        Ok(found.map(|row| UserId::known(row.get("user_id"))))
    }

    async fn end_sign_in(&mut self, token: &SignInToken) -> Result<Option<UserId>, StoreError> {
        let ended = sqlx::query("DELETE FROM sign_ins WHERE fingerprint = ? RETURNING user_id")
            .bind(secrets::fingerprint(&token.0))
            .fetch_optional(self.connection())
            .await
            .map_err(unavailable)?;

        Ok(ended.map(|row| UserId::known(row.get("user_id"))))
    }

    async fn end_every_sign_in(&mut self, user: &UserId) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM sign_ins WHERE user_id = ?")
            .bind(user.as_str())
            .execute(self.connection())
            .await
            .map_err(unavailable)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::store::a_temporary_store;
    use crate::configuration::users::{NewUser, Users};

    async fn a_user(transaction: &mut Transaction, username: &str) -> UserId {
        transaction
            .create_user(NewUser {
                username: username.to_owned(),
                password_hash: None,
                is_system_administrator: false,
            })
            .await
            .expect("the user to be created")
    }

    #[tokio::test]
    async fn a_token_signs_in_whoever_it_was_opened_for() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = a_user(&mut transaction, "flight").await;

        let token = transaction
            .open_sign_in(&user)
            .await
            .expect("the sign-in to open");

        assert_eq!(
            transaction
                .holder_of(&token)
                .await
                .expect("the read to answer"),
            Some(user)
        );
    }

    #[tokio::test]
    async fn the_token_carries_nothing_but_itself() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = a_user(&mut transaction, "flight").await;

        let token = transaction
            .open_sign_in(&user)
            .await
            .expect("the sign-in to open");

        assert!(
            !token.as_str().contains("flight"),
            "the token names the user"
        );
        assert!(
            !token.as_str().contains(user.as_str()),
            "the token carries the internal id"
        );
        assert_eq!(format!("{token:?}"), "SignInToken(withheld)");
    }

    #[tokio::test]
    async fn the_store_holds_a_fingerprint_rather_than_the_token() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = a_user(&mut transaction, "flight").await;
        let token = transaction
            .open_sign_in(&user)
            .await
            .expect("the sign-in to open");

        let held: Vec<String> = sqlx::query_scalar("SELECT fingerprint FROM sign_ins")
            .fetch_all(transaction.connection())
            .await
            .expect("the rows to be readable");

        assert_eq!(held.len(), 1);
        assert_ne!(held[0], token.as_str(), "the token itself is in the store");
    }

    #[tokio::test]
    async fn a_token_nobody_was_given_signs_nobody_in() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");

        let holder = transaction
            .holder_of(&SignInToken::presented("guessed".to_owned()))
            .await
            .expect("the read to answer");

        assert_eq!(holder, None);
    }

    #[tokio::test]
    async fn ending_a_sign_in_says_whose_it_was_and_leaves_the_token_signing_nobody_in() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = a_user(&mut transaction, "flight").await;
        let token = transaction
            .open_sign_in(&user)
            .await
            .expect("the sign-in to open");

        let ended = transaction
            .end_sign_in(&token)
            .await
            .expect("the sign-in to end");

        assert_eq!(ended, Some(user));
        assert_eq!(
            transaction
                .holder_of(&token)
                .await
                .expect("the read to answer"),
            None
        );
        assert_eq!(
            transaction
                .end_sign_in(&token)
                .await
                .expect("the read to answer"),
            None,
            "a sign-in ended twice"
        );
    }

    #[tokio::test]
    async fn one_user_may_be_signed_in_on_several_machines_and_ends_them_one_at_a_time() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = a_user(&mut transaction, "flight").await;
        let console = transaction.open_sign_in(&user).await.expect("one sign-in");
        let laptop = transaction.open_sign_in(&user).await.expect("another");

        transaction
            .end_sign_in(&console)
            .await
            .expect("the sign-in to end");

        assert_eq!(
            transaction
                .holder_of(&laptop)
                .await
                .expect("the read to answer"),
            Some(user)
        );
    }

    #[tokio::test]
    async fn ending_every_sign_in_leaves_the_user_signed_in_nowhere() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = a_user(&mut transaction, "flight").await;
        let elsewhere = a_user(&mut transaction, "capcom").await;
        let console = transaction.open_sign_in(&user).await.expect("one sign-in");
        let laptop = transaction.open_sign_in(&user).await.expect("another");
        let untouched = transaction
            .open_sign_in(&elsewhere)
            .await
            .expect("somebody else's");

        transaction
            .end_every_sign_in(&user)
            .await
            .expect("the sign-ins to end");

        for ended in [&console, &laptop] {
            assert_eq!(
                transaction
                    .holder_of(ended)
                    .await
                    .expect("the read to answer"),
                None
            );
        }
        assert_eq!(
            transaction
                .holder_of(&untouched)
                .await
                .expect("the read to answer"),
            Some(elsewhere),
            "somebody else was signed out too"
        );
    }

    /// There is no state in which a deleted user is signed in. Deleting is #31's operation;
    /// what is checked here is that the store leaves nothing behind when it happens.
    #[tokio::test]
    async fn deleting_a_user_ends_their_sign_ins() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = a_user(&mut transaction, "flight").await;
        let token = transaction
            .open_sign_in(&user)
            .await
            .expect("the sign-in to open");

        sqlx::query("DELETE FROM users WHERE id = ?")
            .bind(user.as_str())
            .execute(transaction.connection())
            .await
            .expect("the user to be deleted");

        assert_eq!(
            transaction
                .holder_of(&token)
                .await
                .expect("the read to answer"),
            None
        );
    }
}
