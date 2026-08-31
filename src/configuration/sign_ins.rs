//! Sign-ins: what a signed-in browser is holding, and who it belongs to.
//!
//! A sign-in is durable. VoxLoop's lifetime table (v1 §2) has occupancy ending at a server
//! restart and the **sign-in surviving** it, which is what puts sign-ins in the store rather
//! than with the live state the state authority holds and discards.
//!
//! What is stored is a fingerprint of the token, never the token: the store is one file a
//! deployment is obliged to back up, and a backup should not be a drawer full of usable
//! sign-ins.

use std::time::Duration;

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

/// How finely the clock is kept.
///
/// The window it feeds is 24 hours, so a minute is already far finer than anything measured
/// against it — and it is what keeps a burst of requests from being a burst of writes.
const TO_THE_MINUTE: Duration = Duration::from_secs(60);

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

    /// Note that whoever holds this sign-in has just done something deliberate.
    ///
    /// It is what the 24-hour window is measured from (v1 §2). *Deliberate* is ADR-0016's
    /// notion, unchanged: something the person did, never something their browser did on
    /// their behalf and never the server pushing at them — a console left open on a desk
    /// has done nothing, which is the whole point of the window.
    ///
    /// It is written no more often than [`TO_THE_MINUTE`], because the window is a day and
    /// recording it to the minute is four orders of magnitude finer than anything turns on.
    /// Without that, a page of ten reads would be ten write transactions saying the same
    /// thing.
    async fn note_a_deliberate_act(&mut self, token: &SignInToken) -> Result<(), StoreError>;

    /// End every sign-in that has seen no deliberate act for `idle_for`, except `spared`.
    ///
    /// The exceptions are the sign-ins that hold a session, which the state authority
    /// answers for: **the clock runs only in the lobby** (ADR-0023), and this is where that
    /// rule is applied. They are passed in as values rather than looked up, because live
    /// state and durable state meet by handing each other data and never by reaching across
    /// ([ADR-0039]).
    ///
    /// Answers with whoever was signed out, so the act can be recorded against them.
    ///
    /// [ADR-0039]: ../../../docs/adr/0039-live-state-is-in-process-behind-one-state-authority.md
    async fn end_sign_ins_idle_for(
        &mut self,
        idle_for: Duration,
        spared: &[SignInToken],
    ) -> Result<Vec<UserId>, StoreError>;
}

#[async_trait]
impl SignIns for Transaction {
    async fn open_sign_in(&mut self, user: &UserId) -> Result<SignInToken, StoreError> {
        let token = SignInToken(secrets::unguessable());

        // Starting is itself the first deliberate act, so the window opens from here rather
        // than from nothing.
        let at = now();
        sqlx::query(
            "INSERT INTO sign_ins (fingerprint, user_id, started_at, last_active_at) \
             VALUES (?, ?, ?, ?)",
        )
        .bind(secrets::fingerprint(&token.0))
        .bind(user.as_str())
        .bind(at)
        .bind(at)
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

    async fn note_a_deliberate_act(&mut self, token: &SignInToken) -> Result<(), StoreError> {
        let at = now();

        sqlx::query(
            "UPDATE sign_ins SET last_active_at = ? WHERE fingerprint = ? AND last_active_at < ?",
        )
        .bind(at)
        .bind(secrets::fingerprint(&token.0))
        .bind(at - milliseconds(TO_THE_MINUTE))
        .execute(self.connection())
        .await
        .map_err(unavailable)?;

        Ok(())
    }

    async fn end_sign_ins_idle_for(
        &mut self,
        idle_for: Duration,
        spared: &[SignInToken],
    ) -> Result<Vec<UserId>, StoreError> {
        let cutoff = now().saturating_sub(milliseconds(idle_for));
        // The spared are matched the way every other sign-in is: by fingerprint, so a token
        // held live in memory is compared against the store without either side handing the
        // other something it could sign in with.
        let held: Vec<String> = spared
            .iter()
            .map(|token| secrets::fingerprint(&token.0))
            .collect();

        // Read the idle ones, then end the ones nobody is sparing. The obvious statement is
        // one `DELETE` with the exceptions in a `NOT IN`, which means building SQL around
        // how many there are; this reads the same and is built out of literal statements.
        // The set is every sign-in nobody has touched for a day, so it is small by
        // construction.
        let idle =
            sqlx::query("SELECT fingerprint, user_id FROM sign_ins WHERE last_active_at <= ?")
                .bind(cutoff)
                .fetch_all(self.connection())
                .await
                .map_err(unavailable)?;

        let mut ended = Vec::new();
        for row in idle {
            let fingerprint: String = row.get("fingerprint");
            if held.contains(&fingerprint) {
                continue;
            }

            sqlx::query("DELETE FROM sign_ins WHERE fingerprint = ?")
                .bind(&fingerprint)
                .execute(self.connection())
                .await
                .map_err(unavailable)?;
            ended.push(UserId::known(row.get("user_id")));
        }

        Ok(ended)
    }
}

#[cfg(test)]
impl Transaction {
    /// Push a sign-in's clock back, the way time passing would.
    ///
    /// Tests only, and there is deliberately no product operation like it: nothing moves a
    /// clock backwards, and the store holds when a sign-in was last active and nothing else,
    /// so this is the whole of what ageing one is.
    pub(crate) async fn a_sign_in_has_been_idle_for(
        &mut self,
        token: &SignInToken,
        since: Duration,
    ) -> Result<(), StoreError> {
        sqlx::query("UPDATE sign_ins SET last_active_at = ? WHERE fingerprint = ?")
            .bind(now() - milliseconds(since))
            .bind(secrets::fingerprint(&token.0))
            .execute(self.connection())
            .await
            .map_err(unavailable)?;

        Ok(())
    }
}

/// A window as the store holds time: milliseconds, saturating rather than wrapping.
fn milliseconds(window: Duration) -> i64 {
    i64::try_from(window.as_millis()).unwrap_or(i64::MAX)
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

    /// The window is measured from the last deliberate act, and a sign-in that has done
    /// nothing since it started is measured from starting.
    #[tokio::test]
    async fn a_sign_in_idle_past_the_window_ends_and_a_fresh_one_stands() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let abandoned = a_user(&mut transaction, "flight").await;
        let working = a_user(&mut transaction, "capcom").await;
        let stale = transaction
            .open_sign_in(&abandoned)
            .await
            .expect("the sign-in to open");
        let fresh = transaction
            .open_sign_in(&working)
            .await
            .expect("the sign-in to open");
        idle_for(&mut transaction, &stale, Duration::from_secs(25 * 60 * 60)).await;

        let ended = transaction
            .end_sign_ins_idle_for(Duration::from_secs(24 * 60 * 60), &[])
            .await
            .expect("the sweep to answer");

        assert_eq!(ended, vec![abandoned]);
        assert_eq!(
            transaction
                .holder_of(&stale)
                .await
                .expect("the read to answer"),
            None
        );
        assert_eq!(
            transaction
                .holder_of(&fresh)
                .await
                .expect("the read to answer"),
            Some(working),
            "a sign-in that had done something recently was swept up with the abandoned one"
        );
    }

    /// A deliberate act restarts the window, which is what keeps somebody who is using
    /// VoxLoop signed in without anything having to be renewed.
    #[tokio::test]
    async fn a_deliberate_act_restarts_the_window() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = a_user(&mut transaction, "flight").await;
        let token = transaction
            .open_sign_in(&user)
            .await
            .expect("the sign-in to open");
        idle_for(&mut transaction, &token, Duration::from_secs(25 * 60 * 60)).await;

        transaction
            .note_a_deliberate_act(&token)
            .await
            .expect("the act to be noted");

        let ended = transaction
            .end_sign_ins_idle_for(Duration::from_secs(24 * 60 * 60), &[])
            .await
            .expect("the sweep to answer");
        assert!(ended.is_empty(), "a sign-in in use was ended");
        assert_eq!(
            transaction
                .holder_of(&token)
                .await
                .expect("the read to answer"),
            Some(user)
        );
    }

    /// The clock runs only in the lobby (ADR-0023): a sign-in holding a session is spared
    /// however long it has been since anybody touched it, so an operator holding a role
    /// through a thirty-hour incident is not signed out for failing to click anything.
    #[tokio::test]
    async fn a_sign_in_holding_a_session_is_spared_however_long_it_has_been_idle() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let occupant = a_user(&mut transaction, "flight").await;
        let elsewhere = a_user(&mut transaction, "capcom").await;
        let holding_a_role = transaction
            .open_sign_in(&occupant)
            .await
            .expect("the sign-in to open");
        let in_the_lobby = transaction
            .open_sign_in(&elsewhere)
            .await
            .expect("the sign-in to open");
        let thirty_hours = Duration::from_secs(30 * 60 * 60);
        idle_for(&mut transaction, &holding_a_role, thirty_hours).await;
        idle_for(&mut transaction, &in_the_lobby, thirty_hours).await;

        let ended = transaction
            .end_sign_ins_idle_for(
                Duration::from_secs(24 * 60 * 60),
                std::slice::from_ref(&holding_a_role),
            )
            .await
            .expect("the sweep to answer");

        assert_eq!(ended, vec![elsewhere]);
        assert_eq!(
            transaction
                .holder_of(&holding_a_role)
                .await
                .expect("the read to answer"),
            Some(occupant)
        );
        assert_eq!(
            transaction
                .holder_of(&in_the_lobby)
                .await
                .expect("the read to answer"),
            None,
            "a sign-in standing in the lobby outlasted the window because another one held a \
             session"
        );
    }

    async fn idle_for(transaction: &mut Transaction, token: &SignInToken, since: Duration) {
        transaction
            .a_sign_in_has_been_idle_for(token, since)
            .await
            .expect("the clock to be moved back");
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
