//! Enrolment codes: the credential that replaces every link an email would have carried.
//!
//! VoxLoop has no mail path, so a password is set by redeeming a **single-use expiring code
//! issued by an administrator and handed over out of band** ([ADR-0025]). A reset is the same
//! act again. There is no self-registration and no self-service reset, here or anywhere.
//!
//! It sits beside [`super::sign_ins`] rather than in Identity for the same reason a sign-in
//! token does: the code is opaque, it means nothing but *this user*, and what makes it a
//! credential is being unguessable and short-lived rather than anything about how a password
//! is checked. Identity hashes the password the redemption sets; it never sees the code.
//!
//! What is stored is a fingerprint of the code and never the code itself. **Spending a code
//! is deleting it**, so single use is one statement rather than a flag two callers could
//! read differently, and an outstanding code is a row that has not expired.
//!
//! [ADR-0025]: ../../../docs/adr/0025-credentials-are-administered-because-there-is-no-email.md

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use sqlx::Row;

use super::store::{StoreError, Transaction, now, unavailable};
use super::users::UserId;
use crate::secrets;

/// How long a code is good for.
///
/// It has to survive being handed over out of band, which in an operations centre means
/// surviving a shift rotation: the administrator who issues one and the operator who spends
/// it are routinely not in the building on the same day. A week is long enough for that and
/// far short of a code sitting in a chat log being a standing credential.
const GOOD_FOR: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// The value handed over, and the whole of what is handed over.
///
/// It carries no claims — not the username, not the expiry — for the reason a sign-in token
/// carries none: everything about what it enrols is read from the store when it is spent.
#[derive(Clone)]
pub(crate) struct EnrolmentCode(String);

impl EnrolmentCode {
    /// Take a code as somebody presented it.
    pub(crate) fn presented(value: String) -> Self {
        Self(value)
    }

    /// The value to hand the administrator, who hands it to the user. Every other use of
    /// this is a mistake.
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// A code is a live credential, and a credential that turns up in a log is spent.
impl std::fmt::Debug for EnrolmentCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("EnrolmentCode(withheld)")
    }
}

/// A code that has been issued and not yet spent, as anything but the code itself.
///
/// This is what the admin console reads: an administrator needs to know a code is out there
/// and when it stops being good, and must never be able to read one back — a code readable
/// twice is a code that was never single-use in the sense that matters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Outstanding {
    /// Milliseconds since the Unix epoch.
    pub(crate) expires_at: i64,
}

/// A code just issued: the value to hand over, and what issuing it invalidated.
///
/// The replaced code is here because issuing is an audited administration write and an entry
/// records what the write changed. Reissuing against a user who already had one outstanding
/// is the ordinary case — an administrator reissues because the first was mislaid — and the
/// entry should say that the first one stopped working.
pub(crate) struct Issued {
    pub(crate) code: EnrolmentCode,
    pub(crate) outstanding: Outstanding,
    pub(crate) replaced: Option<Outstanding>,
}

/// Enrolment codes, as domain operations rather than queries.
#[async_trait]
pub(crate) trait Enrolment {
    /// Issue a code against a user, invalidating whatever they had outstanding.
    ///
    /// The lifetime is not a parameter. A credential's expiry is a property of the
    /// credential rather than something each caller chooses, and a caller that could choose
    /// would eventually choose *never*.
    async fn issue_enrolment_code(&mut self, user: &UserId) -> Result<Issued, StoreError>;

    /// Spend a code, answering with the user it enrols, or with nobody.
    ///
    /// Nobody is the one answer for a code that was never issued, one already spent and one
    /// that has expired. Which of the three it was is not something an unauthenticated
    /// caller is entitled to learn, and the store has nothing left to tell them anyway.
    async fn spend_enrolment_code(
        &mut self,
        code: &EnrolmentCode,
    ) -> Result<Option<UserId>, StoreError>;

    /// Every code that is outstanding right now, by the user it enrols.
    ///
    /// The console reads this to say *a code is out there* beside an account with no
    /// password, so that an administrator who has already issued one does not issue a
    /// second and leave the first in somebody's hand.
    async fn outstanding_enrolments(&mut self) -> Result<HashMap<UserId, Outstanding>, StoreError>;
}

#[async_trait]
impl Enrolment for Transaction {
    async fn issue_enrolment_code(&mut self, user: &UserId) -> Result<Issued, StoreError> {
        let issued_at = now();
        let expires_at =
            issued_at.saturating_add(i64::try_from(GOOD_FOR.as_millis()).unwrap_or(i64::MAX));

        // Expired rows say exactly what an absent one says, so they go on the way past. A
        // table full of codes nobody can spend is a graveyard to be read around later.
        sqlx::query("DELETE FROM enrolment_codes WHERE expires_at <= ?")
            .bind(issued_at)
            .execute(self.connection())
            .await
            .map_err(unavailable)?;

        let replaced =
            sqlx::query("DELETE FROM enrolment_codes WHERE user_id = ? RETURNING expires_at")
                .bind(user.as_str())
                .fetch_optional(self.connection())
                .await
                .map_err(unavailable)?
                .map(|row| Outstanding {
                    expires_at: row.get("expires_at"),
                });

        let code = EnrolmentCode(secrets::unguessable());

        sqlx::query(
            "INSERT INTO enrolment_codes (fingerprint, user_id, issued_at, expires_at) \
             VALUES (?, ?, ?, ?)",
        )
        .bind(secrets::fingerprint(&code.0))
        .bind(user.as_str())
        .bind(issued_at)
        .bind(expires_at)
        .execute(self.connection())
        .await
        .map_err(unavailable)?;

        Ok(Issued {
            code,
            outstanding: Outstanding { expires_at },
            replaced,
        })
    }

    async fn spend_enrolment_code(
        &mut self,
        code: &EnrolmentCode,
    ) -> Result<Option<UserId>, StoreError> {
        // Reading and deleting in one statement is what makes single use single use: two
        // redemptions arriving together cannot both find the row, whatever the caller does
        // with the answer.
        let spent = sqlx::query(
            "DELETE FROM enrolment_codes WHERE fingerprint = ? AND expires_at > ? \
             RETURNING user_id",
        )
        .bind(secrets::fingerprint(&code.0))
        .bind(now())
        .fetch_optional(self.connection())
        .await
        .map_err(unavailable)?;

        Ok(spent.map(|row| UserId::known(row.get("user_id"))))
    }

    async fn outstanding_enrolments(&mut self) -> Result<HashMap<UserId, Outstanding>, StoreError> {
        let rows =
            sqlx::query("SELECT user_id, expires_at FROM enrolment_codes WHERE expires_at > ?")
                .bind(now())
                .fetch_all(self.connection())
                .await
                .map_err(unavailable)?;

        Ok(rows
            .into_iter()
            .map(|row| {
                (
                    UserId::known(row.get("user_id")),
                    Outstanding {
                        expires_at: row.get("expires_at"),
                    },
                )
            })
            .collect())
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

    /// Age a user's outstanding code past its expiry, as a week of waiting would.
    async fn let_it_expire(transaction: &mut Transaction, user: &UserId) {
        sqlx::query("UPDATE enrolment_codes SET expires_at = ? WHERE user_id = ?")
            .bind(now() - 1)
            .bind(user.as_str())
            .execute(transaction.connection())
            .await
            .expect("the code to be aged");
    }

    #[tokio::test]
    async fn a_code_enrols_whoever_it_was_issued_against() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = a_user(&mut transaction, "flight").await;

        let issued = transaction
            .issue_enrolment_code(&user)
            .await
            .expect("the code to be issued");

        assert_eq!(
            transaction
                .spend_enrolment_code(&issued.code)
                .await
                .expect("the code to be spendable"),
            Some(user)
        );
    }

    #[tokio::test]
    async fn a_code_is_good_once() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = a_user(&mut transaction, "flight").await;
        let issued = transaction
            .issue_enrolment_code(&user)
            .await
            .expect("the code to be issued");

        transaction
            .spend_enrolment_code(&issued.code)
            .await
            .expect("the code to be spendable");

        assert_eq!(
            transaction
                .spend_enrolment_code(&issued.code)
                .await
                .expect("the read to answer"),
            None,
            "a code was spent twice"
        );
    }

    #[tokio::test]
    async fn a_code_nobody_issued_enrols_nobody() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");

        assert_eq!(
            transaction
                .spend_enrolment_code(&EnrolmentCode::presented("guessed".to_owned()))
                .await
                .expect("the read to answer"),
            None
        );
    }

    #[tokio::test]
    async fn an_expired_code_enrols_nobody_and_is_outstanding_to_nobody() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = a_user(&mut transaction, "flight").await;
        let issued = transaction
            .issue_enrolment_code(&user)
            .await
            .expect("the code to be issued");

        let_it_expire(&mut transaction, &user).await;

        assert_eq!(
            transaction
                .spend_enrolment_code(&issued.code)
                .await
                .expect("the read to answer"),
            None
        );
        assert!(
            transaction
                .outstanding_enrolments()
                .await
                .expect("the read to answer")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn issuing_a_second_code_invalidates_the_first_and_says_what_it_replaced() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = a_user(&mut transaction, "flight").await;
        let first = transaction
            .issue_enrolment_code(&user)
            .await
            .expect("a code");

        let second = transaction
            .issue_enrolment_code(&user)
            .await
            .expect("another code");

        assert_eq!(second.replaced, Some(first.outstanding));
        assert_eq!(
            transaction
                .spend_enrolment_code(&first.code)
                .await
                .expect("the read to answer"),
            None,
            "the code that was replaced still works"
        );
        assert_eq!(
            transaction
                .spend_enrolment_code(&second.code)
                .await
                .expect("the code to be spendable"),
            Some(user)
        );
    }

    #[tokio::test]
    async fn the_first_code_a_user_is_issued_replaces_nothing() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = a_user(&mut transaction, "flight").await;

        let issued = transaction
            .issue_enrolment_code(&user)
            .await
            .expect("a code");

        assert_eq!(issued.replaced, None);
    }

    #[tokio::test]
    async fn the_store_holds_a_fingerprint_rather_than_the_code() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = a_user(&mut transaction, "flight").await;
        let issued = transaction
            .issue_enrolment_code(&user)
            .await
            .expect("a code");

        let held: Vec<String> = sqlx::query_scalar("SELECT fingerprint FROM enrolment_codes")
            .fetch_all(transaction.connection())
            .await
            .expect("the rows to be readable");

        assert_eq!(held.len(), 1);
        assert_ne!(
            held[0],
            issued.code.as_str(),
            "the code itself is in the store"
        );
    }

    #[tokio::test]
    async fn a_code_carries_nothing_but_itself() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = a_user(&mut transaction, "flight").await;

        let issued = transaction
            .issue_enrolment_code(&user)
            .await
            .expect("a code");

        assert!(
            !issued.code.as_str().contains(user.as_str()),
            "the code carries the internal id"
        );
        assert_eq!(format!("{:?}", issued.code), "EnrolmentCode(withheld)");
    }

    #[tokio::test]
    async fn says_which_users_have_a_code_outstanding_and_when_it_stops_being_good() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let awaiting = a_user(&mut transaction, "flight").await;
        let enrolled = a_user(&mut transaction, "capcom").await;
        let issued = transaction
            .issue_enrolment_code(&awaiting)
            .await
            .expect("a code");
        let spent = transaction
            .issue_enrolment_code(&enrolled)
            .await
            .expect("a code");
        transaction
            .spend_enrolment_code(&spent.code)
            .await
            .expect("the code to be spendable");

        let outstanding = transaction
            .outstanding_enrolments()
            .await
            .expect("the read to answer");

        assert_eq!(outstanding.get(&awaiting), Some(&issued.outstanding));
        assert_eq!(
            outstanding.get(&enrolled),
            None,
            "a spent code is outstanding"
        );
    }

    #[tokio::test]
    async fn deleting_a_user_takes_their_outstanding_code_with_them() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = a_user(&mut transaction, "flight").await;
        let issued = transaction
            .issue_enrolment_code(&user)
            .await
            .expect("a code");

        transaction
            .delete_user(&user)
            .await
            .expect("the user to be deleted");

        assert_eq!(
            transaction
                .spend_enrolment_code(&issued.code)
                .await
                .expect("the read to answer"),
            None
        );
    }
}
