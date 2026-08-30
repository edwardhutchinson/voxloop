//! The user record, and the repository that holds it.
//!
//! A user record carries three things with strictly separate jobs ([ADR-0024]): an
//! **immutable opaque internal id**, which is the only thing anything else in VoxLoop
//! references; a **mutable username**, which is for humans to type; and a **nullable
//! external identity**, the (issuer, subject) pair, which v1 stores and never writes.
//!
//! Because everything references the id, renaming a user changes nothing else — and that is
//! a property to be tested rather than intended, since a stray join on username works
//! perfectly until the first rename.
//!
//! [ADR-0024]: ../../../docs/adr/0024-identity-is-a-replaceable-front-door.md

use async_trait::async_trait;
use sqlx::Row;

use super::store::{StoreError, Transaction, now, unavailable};
use crate::secrets;

/// The immutable opaque internal id of a user, never reused.
///
/// It is the only thing eligibility, audit, sign-ins and everything else hold, which is what
/// makes a rename safe and what keeps an audit entry correct after the user is deleted.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct UserId(String);

impl UserId {
    /// Take back an id the store, a cookie or a repository already minted.
    pub(super) fn known(id: String) -> Self {
        Self(id)
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// A stored password hash, which this module keeps and never interprets.
///
/// Only Identity knows what is inside one. Configuration's job is to hand it back to the
/// module that can check it ([ADR-0024]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PasswordHash(String);

impl PasswordHash {
    pub(crate) fn already_hashed(hash: String) -> Self {
        Self(hash)
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// The pair naming a user in a customer's identity provider.
///
/// Stored in v1 and written by nothing. Linking a VoxLoop user to an external subject is an
/// explicit administrative act, and an email address is never one ([ADR-0024]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExternalIdentity {
    pub(crate) issuer: String,
    pub(crate) subject: String,
}

/// A user, as everything outside this module sees one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct User {
    pub(crate) id: UserId,
    pub(crate) username: String,
    /// The user-level flag of ADR-0003, held by the person and never by a role.
    pub(crate) is_system_administrator: bool,
    pub(crate) external_identity: Option<ExternalIdentity>,
}

/// A user about to exist.
pub(crate) struct NewUser {
    pub(crate) username: String,
    /// Absent where system administration has created the record and an enrolment code has
    /// yet to set a password on it.
    pub(crate) password_hash: Option<PasswordHash>,
    pub(crate) is_system_administrator: bool,
}

/// What can stop a user record from being written under a given name.
///
/// The two are different in kind, and the type says so rather than leaving it to whoever
/// writes the next `match`: one is a refusal a human acts on by choosing another name, the
/// other is a fault. Folding the first into [`StoreError`] would let a caller who forgot the
/// arm answer "that name is taken" with "VoxLoop could not answer that just now".
#[derive(Debug, thiserror::Error)]
pub(crate) enum NameRefused {
    #[error("the username {username:?} is already taken")]
    Taken { username: String },

    #[error(transparent)]
    Store(#[from] StoreError),
}

/// What a local password check needs, and nothing more.
pub(crate) struct StoredPassword {
    pub(crate) user: UserId,
    pub(crate) hash: PasswordHash,
}

/// The user record, as domain operations rather than queries ([ADR-0038]).
///
/// [ADR-0038]: ../../../docs/adr/0038-sqlite-behind-domain-shaped-repositories.md
#[async_trait]
pub(crate) trait Users {
    /// Create a user, and answer with the id nothing will ever change.
    async fn create_user(&mut self, new: NewUser) -> Result<UserId, NameRefused>;

    /// Read a user by the id that identifies them.
    async fn user(&mut self, id: &UserId) -> Result<Option<User>, StoreError>;

    /// Change the name a user types, leaving everything that references them alone.
    ///
    /// Editing users is #31's operation. It is here now because ADR-0024 requires the
    /// rename-changes-nothing property to be *tested* — a stray join on username works
    /// perfectly until the first rename — and a property with nothing to exercise it is not
    /// tested at all.
    #[allow(dead_code)]
    async fn rename_user(&mut self, id: &UserId, username: &str) -> Result<(), NameRefused>;

    /// The stored password a name resolves to, for whoever is entitled to check it.
    async fn stored_password(
        &mut self,
        username: &str,
    ) -> Result<Option<StoredPassword>, StoreError>;

    /// Whether anybody at all holds the system-administration flag.
    ///
    /// The bootstrap code exists for exactly as long as this answers `false`.
    async fn a_system_administrator_exists(&mut self) -> Result<bool, StoreError>;
}

#[async_trait]
impl Users for Transaction {
    async fn create_user(&mut self, new: NewUser) -> Result<UserId, NameRefused> {
        let id = UserId(secrets::unguessable());

        sqlx::query(
            "INSERT INTO users (id, username, password_hash, is_system_administrator, created_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&id.0)
        .bind(&new.username)
        .bind(new.password_hash.as_ref().map(PasswordHash::as_str))
        .bind(i64::from(new.is_system_administrator))
        .bind(now())
        .execute(self.connection())
        .await
        .map_err(|error| taken_or_unavailable(error, &new.username))?;

        Ok(id)
    }

    async fn user(&mut self, id: &UserId) -> Result<Option<User>, StoreError> {
        let found = sqlx::query(
            "SELECT id, username, is_system_administrator, external_issuer, external_subject \
             FROM users WHERE id = ?",
        )
        .bind(&id.0)
        .fetch_optional(self.connection())
        .await
        .map_err(unavailable)?;

        Ok(found.map(|row| User {
            id: UserId(row.get("id")),
            username: row.get("username"),
            is_system_administrator: row.get::<i64, _>("is_system_administrator") != 0,
            external_identity: row
                .get::<Option<String>, _>("external_issuer")
                .map(|issuer| ExternalIdentity {
                    issuer,
                    subject: row.get("external_subject"),
                }),
        }))
    }

    async fn rename_user(&mut self, id: &UserId, username: &str) -> Result<(), NameRefused> {
        sqlx::query("UPDATE users SET username = ? WHERE id = ?")
            .bind(username)
            .bind(&id.0)
            .execute(self.connection())
            .await
            .map_err(|error| taken_or_unavailable(error, username))?;

        Ok(())
    }

    async fn stored_password(
        &mut self,
        username: &str,
    ) -> Result<Option<StoredPassword>, StoreError> {
        let found = sqlx::query(
            "SELECT id, password_hash FROM users WHERE username = ? AND password_hash IS NOT NULL",
        )
        .bind(username)
        .fetch_optional(self.connection())
        .await
        .map_err(unavailable)?;

        Ok(found.map(|row| StoredPassword {
            user: UserId(row.get("id")),
            hash: PasswordHash(row.get("password_hash")),
        }))
    }

    async fn a_system_administrator_exists(&mut self) -> Result<bool, StoreError> {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE is_system_administrator = 1")
                .fetch_one(self.connection())
                .await
                .map_err(unavailable)?;

        Ok(count > 0)
    }
}

/// Tell a name that is already taken apart from a store that could not answer.
fn taken_or_unavailable(error: sqlx::Error, username: &str) -> NameRefused {
    let taken = error
        .as_database_error()
        .is_some_and(sqlx::error::DatabaseError::is_unique_violation);

    if taken {
        NameRefused::Taken {
            username: username.to_owned(),
        }
    } else {
        NameRefused::Store(unavailable(error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::store::a_temporary_store;

    fn a_new_user(username: &str) -> NewUser {
        NewUser {
            username: username.to_owned(),
            password_hash: Some(PasswordHash::already_hashed(
                "$argon2id$stand-in".to_owned(),
            )),
            is_system_administrator: false,
        }
    }

    #[tokio::test]
    async fn creates_a_user_and_reads_it_back() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");

        let id = transaction
            .create_user(a_new_user("flight"))
            .await
            .expect("the user to be created");
        let read = transaction.user(&id).await.expect("the read to answer");

        assert_eq!(
            read,
            Some(User {
                id,
                username: "flight".to_owned(),
                is_system_administrator: false,
                external_identity: None,
            })
        );
    }

    #[tokio::test]
    async fn mints_an_id_that_is_neither_the_name_nor_the_one_before_it() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");

        let first = transaction
            .create_user(a_new_user("flight"))
            .await
            .expect("the first user");
        let second = transaction
            .create_user(a_new_user("capcom"))
            .await
            .expect("the second user");

        assert_ne!(first, second);
        assert!(!first.as_str().contains("flight"), "the id names the user");
    }

    #[tokio::test]
    async fn refuses_a_username_already_taken_whatever_its_case() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        transaction
            .create_user(a_new_user("flight"))
            .await
            .expect("the first user");

        let refusal = transaction.create_user(a_new_user("FLIGHT")).await;

        assert!(
            matches!(refusal, Err(NameRefused::Taken { .. })),
            "expected the name to be refused, got {refusal:?}",
        );
    }

    #[tokio::test]
    async fn renaming_a_user_leaves_the_id_everything_references_alone() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let id = transaction
            .create_user(a_new_user("flight"))
            .await
            .expect("the user to be created");

        transaction
            .rename_user(&id, "flight-director")
            .await
            .expect("the rename to land");

        let read = transaction.user(&id).await.expect("the read to answer");
        assert_eq!(read.expect("the user").username, "flight-director");
        assert!(
            transaction
                .stored_password("flight")
                .await
                .expect("the read to answer")
                .is_none(),
            "the old name still resolves to a credential"
        );
        assert_eq!(
            transaction
                .stored_password("flight-director")
                .await
                .expect("the read to answer")
                .expect("the credential")
                .user,
            id,
            "the new name resolves to a different user"
        );
    }

    #[tokio::test]
    async fn a_name_resolves_to_the_stored_password_whatever_its_case() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let id = transaction
            .create_user(a_new_user("flight"))
            .await
            .expect("the user to be created");

        let found = transaction
            .stored_password("FLIGHT")
            .await
            .expect("the read to answer")
            .expect("the credential");

        assert_eq!(found.user, id);
        assert_eq!(found.hash.as_str(), "$argon2id$stand-in");
    }

    #[tokio::test]
    async fn a_user_with_no_password_yet_resolves_to_no_credential() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        transaction
            .create_user(NewUser {
                username: "enrolling".to_owned(),
                password_hash: None,
                is_system_administrator: false,
            })
            .await
            .expect("the user to be created");

        let found = transaction
            .stored_password("enrolling")
            .await
            .expect("the read to answer");

        assert!(found.is_none(), "a record with no password answered one");
    }

    #[tokio::test]
    async fn a_name_nobody_holds_resolves_to_nothing() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");

        let found = transaction
            .stored_password("nobody")
            .await
            .expect("the read to answer");

        assert!(found.is_none());
    }

    #[tokio::test]
    async fn a_system_administrator_exists_only_once_one_does() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        assert!(
            !transaction
                .a_system_administrator_exists()
                .await
                .expect("the read to answer")
        );

        transaction
            .create_user(a_new_user("flight"))
            .await
            .expect("an ordinary user");
        assert!(
            !transaction
                .a_system_administrator_exists()
                .await
                .expect("the read to answer"),
            "an ordinary user counted as a system administrator"
        );

        transaction
            .create_user(NewUser {
                is_system_administrator: true,
                ..a_new_user("root")
            })
            .await
            .expect("an administrator");

        assert!(
            transaction
                .a_system_administrator_exists()
                .await
                .expect("the read to answer")
        );
    }

    /// Email is never an identity key and never a join key ([ADR-0024]). The cheapest way to
    /// keep that true is to have nowhere to put one, and this is what checks it stays true.
    ///
    /// [ADR-0024]: ../../../docs/adr/0024-identity-is-a-replaceable-front-door.md
    #[tokio::test]
    async fn no_table_in_the_store_holds_an_email_address() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");

        let columns: Vec<String> = sqlx::query_scalar(
            "SELECT t.name || '.' || c.name FROM sqlite_master AS t \
             JOIN pragma_table_info(t.name) AS c WHERE t.type = 'table'",
        )
        .fetch_all(transaction.connection())
        .await
        .expect("the schema to be readable");

        let email: Vec<&String> = columns
            .iter()
            .filter(|column| column.to_lowercase().contains("mail"))
            .collect();
        assert!(email.is_empty(), "the schema holds {email:?}");
    }

    /// The (issuer, subject) pair is stored in v1 and written by nothing, so what this holds
    /// the schema to is that it is there, nullable, and empty on every user v1 creates.
    #[tokio::test]
    async fn a_user_created_in_v1_has_no_external_identity() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");

        let id = transaction
            .create_user(a_new_user("flight"))
            .await
            .expect("the user to be created");

        assert_eq!(
            transaction
                .user(&id)
                .await
                .expect("the read to answer")
                .expect("the user")
                .external_identity,
            None
        );
    }

    #[tokio::test]
    async fn an_external_identity_is_read_back_as_the_pair_it_is() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let id = transaction
            .create_user(a_new_user("flight"))
            .await
            .expect("the user to be created");

        // Nothing in v1 writes one, so this is the administrative act of a later version,
        // standing in to prove the column pair carries what ADR-0024 says it carries.
        sqlx::query("UPDATE users SET external_issuer = ?, external_subject = ? WHERE id = ?")
            .bind("https://identity.example")
            .bind("8f2c")
            .bind(id.as_str())
            .execute(transaction.connection())
            .await
            .expect("the link to land");

        assert_eq!(
            transaction
                .user(&id)
                .await
                .expect("the read to answer")
                .expect("the user")
                .external_identity,
            Some(ExternalIdentity {
                issuer: "https://identity.example".to_owned(),
                subject: "8f2c".to_owned(),
            })
        );
    }
}
