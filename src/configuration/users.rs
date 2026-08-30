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

use super::sign_ins::SignIns;
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

    /// Take an id as a caller presented it, in a path or a request body.
    ///
    /// It is opaque to everything outside this module, so there is nothing to validate here:
    /// an id nobody holds reads back as no user, which is the answer the caller needed.
    pub(crate) fn presented(id: String) -> Self {
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
    /// A deliberate administrative act, never a consequence of failed attempts ([ADR-0025]).
    ///
    /// [ADR-0025]: ../../../docs/adr/0025-credentials-are-administered-because-there-is-no-email.md
    pub(crate) is_locked: bool,
    /// Whether a password has been set, never anything about what it is.
    ///
    /// A user created by system administration has none until an enrolment code sets one, so
    /// this is an ordinary state rather than a fault — and it is what makes a forced password
    /// reset visible in the audit log as a change rather than as two identical lines.
    pub(crate) has_password: bool,
    pub(crate) external_identity: Option<ExternalIdentity>,
}

/// A user record before and after a write to it.
///
/// Every configuration change is audited with **before and after** (v1 §12), so a write
/// answers with both rather than leaving the caller to read around it — which would be two
/// more reads and a window in which the answer is assembled from three different moments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Change {
    pub(crate) before: User,
    /// Absent on a deletion, which is the whole of what a deletion says.
    pub(crate) after: Option<User>,
}

impl Change {
    /// Two writes read as the one change they were asked for.
    ///
    /// An edit that renames and takes the flag away is one act by one administrator, and the
    /// log records where the record started and where it ended rather than the step between.
    pub(crate) fn then(self, next: Self) -> Self {
        Self {
            before: self.before,
            after: next.after,
        }
    }
}

/// A user about to exist.
pub(crate) struct NewUser {
    pub(crate) username: String,
    /// Absent where system administration has created the record and an enrolment code has
    /// yet to set a password on it.
    pub(crate) password_hash: Option<PasswordHash>,
    pub(crate) is_system_administrator: bool,
}

/// What can stop a user administration write.
///
/// The refusals and the fault are different in kind, and the type says so rather than
/// leaving it to whoever writes the next `match`: the first two are refusals a human acts
/// on — by choosing another name, or by promoting somebody before demoting themselves — and
/// the third is a fault. Folding a refusal into [`StoreError`] would let a caller who forgot
/// the arm answer "that name is taken" with "VoxLoop could not answer that just now".
#[derive(Debug, thiserror::Error)]
pub(crate) enum AdministrationRefused {
    #[error("the username {username:?} is already taken")]
    NameTaken { username: String },

    /// The last system administrator cannot be removed (v1 §2). Clearing the flag on,
    /// locking or deleting the final one is refused, because each of the three leaves a
    /// deployment nobody can administer and only shell access to the box can recover it.
    ///
    /// *Final* counts flag holders and nothing else. Narrowing it to the ones who could
    /// sign in today reads as an improvement and is a hole: an administrator who stops
    /// counting is one the next call may delete, and a box can be emptied of them one act
    /// at a time.
    #[error("that is the last system administrator this deployment can be administered by")]
    LastSystemAdministrator,

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
    async fn create_user(&mut self, new: NewUser) -> Result<UserId, AdministrationRefused>;

    /// Read a user by the id that identifies them.
    async fn user(&mut self, id: &UserId) -> Result<Option<User>, StoreError>;

    /// Read a user by the name a human types, however it was capitalised.
    ///
    /// The id is what everything else holds, so this exists for the one caller that starts
    /// from something a person typed: the on-box CLI, which is handed a username at a shell
    /// and has no id to be given.
    async fn user_named(&mut self, username: &str) -> Result<Option<User>, StoreError>;

    /// Every user on the deployment, by name.
    ///
    /// The console reads one row at a time ([ADR-0015]) and this is the list it picks from;
    /// there is no paging, because a deployment's user count is bounded by the size of an
    /// operations centre.
    ///
    /// [ADR-0015]: ../../../docs/adr/0015-the-admin-console-reads-one-row-at-a-time.md
    async fn users(&mut self) -> Result<Vec<User>, StoreError>;

    /// Change the name a user types, leaving everything that references them alone.
    ///
    /// Answers with the change, or with nothing where the id names nobody.
    async fn rename_user(
        &mut self,
        id: &UserId,
        username: &str,
    ) -> Result<Option<Change>, AdministrationRefused>;

    /// Give or take away the system-administration flag.
    ///
    /// It is held by the person and never by a role ([ADR-0003]), which is why this is one
    /// operation on a user rather than a cell on the grid.
    ///
    /// [ADR-0003]: ../../../docs/adr/0003-operational-authority-follows-the-role.md
    async fn set_system_administration(
        &mut self,
        id: &UserId,
        held: bool,
    ) -> Result<Option<Change>, AdministrationRefused>;

    /// Lock or unlock an account.
    ///
    /// Locking **ends the user's sign-in** as part of the same act (v1 §2's lifetime table),
    /// which is why it is one operation rather than a flag the caller remembers to follow up.
    /// Their session ends with it, on the live side, where sessions are.
    async fn set_account_lock(
        &mut self,
        id: &UserId,
        locked: bool,
    ) -> Result<Option<Change>, AdministrationRefused>;

    /// Take away the password, so the user has none until an enrolment code sets one.
    ///
    /// This is the store half of a forced password reset. Like a lock, it **ends every
    /// sign-in the user holds** rather than leaving one open against a credential that no
    /// longer exists.
    async fn clear_password(&mut self, id: &UserId) -> Result<Option<Change>, StoreError>;

    /// Set the password, whether the user had one or not.
    ///
    /// This is the store half of both acts that set one: redeeming an enrolment code, and a
    /// signed-in user changing their own. It is deliberately one operation, because the two
    /// differ only in what entitled the caller to it and not at all in what lands.
    ///
    /// It **leaves every sign-in alone**, deliberately, because the two acts that set a
    /// password disagree about that and the store is not where the disagreement is settled:
    /// an operator on the air changing their own password keeps their session (v1 §2), and a
    /// redemption ends the sign-ins standing against the credential it replaced.
    async fn set_password(
        &mut self,
        id: &UserId,
        hashed: PasswordHash,
    ) -> Result<Option<Change>, StoreError>;

    /// The password this user holds, for whoever is entitled to check it.
    ///
    /// Nobody is the answer for a user who has none yet, which is what makes re-presenting
    /// the current password impossible rather than skippable for an account awaiting
    /// enrolment.
    async fn password_held_by(&mut self, id: &UserId) -> Result<Option<PasswordHash>, StoreError>;

    /// Delete a user, and everything that is only about them.
    ///
    /// Their sign-ins go with them — there is no state in which a deleted user is signed in
    /// — and their audit entries stay, readable and attributed ([ADR-0028]).
    ///
    /// [ADR-0028]: ../../../docs/adr/0028-the-audit-log-records-decisions-not-traffic.md
    async fn delete_user(&mut self, id: &UserId) -> Result<Option<Change>, AdministrationRefused>;

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
    async fn create_user(&mut self, new: NewUser) -> Result<UserId, AdministrationRefused> {
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
            "SELECT id, username, is_system_administrator, is_locked, password_hash, \
             external_issuer, external_subject FROM users WHERE id = ?",
        )
        .bind(&id.0)
        .fetch_optional(self.connection())
        .await
        .map_err(unavailable)?;

        Ok(found.as_ref().map(a_user))
    }

    async fn user_named(&mut self, username: &str) -> Result<Option<User>, StoreError> {
        let found = sqlx::query(
            "SELECT id, username, is_system_administrator, is_locked, password_hash, \
             external_issuer, external_subject FROM users WHERE username = ?",
        )
        .bind(username)
        .fetch_optional(self.connection())
        .await
        .map_err(unavailable)?;

        Ok(found.as_ref().map(a_user))
    }

    async fn users(&mut self) -> Result<Vec<User>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, username, is_system_administrator, is_locked, password_hash, \
             external_issuer, external_subject FROM users ORDER BY username",
        )
        .fetch_all(self.connection())
        .await
        .map_err(unavailable)?;

        Ok(rows.iter().map(a_user).collect())
    }

    async fn rename_user(
        &mut self,
        id: &UserId,
        username: &str,
    ) -> Result<Option<Change>, AdministrationRefused> {
        let Some(before) = self.user(id).await? else {
            return Ok(None);
        };

        sqlx::query("UPDATE users SET username = ? WHERE id = ?")
            .bind(username)
            .bind(&id.0)
            .execute(self.connection())
            .await
            .map_err(|error| taken_or_unavailable(error, username))?;

        Ok(Some(self.changed(before, id).await?))
    }

    async fn set_system_administration(
        &mut self,
        id: &UserId,
        held: bool,
    ) -> Result<Option<Change>, AdministrationRefused> {
        let Some(before) = self.user(id).await? else {
            return Ok(None);
        };

        if !held && is_the_last_administrator(self, id).await? {
            return Err(AdministrationRefused::LastSystemAdministrator);
        }

        sqlx::query("UPDATE users SET is_system_administrator = ? WHERE id = ?")
            .bind(i64::from(held))
            .bind(&id.0)
            .execute(self.connection())
            .await
            .map_err(unavailable)?;

        Ok(Some(self.changed(before, id).await?))
    }

    async fn set_account_lock(
        &mut self,
        id: &UserId,
        locked: bool,
    ) -> Result<Option<Change>, AdministrationRefused> {
        let Some(before) = self.user(id).await? else {
            return Ok(None);
        };

        if locked && is_the_last_administrator(self, id).await? {
            return Err(AdministrationRefused::LastSystemAdministrator);
        }

        sqlx::query("UPDATE users SET is_locked = ? WHERE id = ?")
            .bind(i64::from(locked))
            .bind(&id.0)
            .execute(self.connection())
            .await
            .map_err(unavailable)?;

        if locked {
            self.end_every_sign_in(id).await?;
        }

        Ok(Some(self.changed(before, id).await?))
    }

    async fn clear_password(&mut self, id: &UserId) -> Result<Option<Change>, StoreError> {
        let Some(before) = self.user(id).await? else {
            return Ok(None);
        };

        sqlx::query("UPDATE users SET password_hash = NULL WHERE id = ?")
            .bind(&id.0)
            .execute(self.connection())
            .await
            .map_err(unavailable)?;

        self.end_every_sign_in(id).await?;

        Ok(Some(self.changed(before, id).await?))
    }

    async fn set_password(
        &mut self,
        id: &UserId,
        hashed: PasswordHash,
    ) -> Result<Option<Change>, StoreError> {
        let Some(before) = self.user(id).await? else {
            return Ok(None);
        };

        sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
            .bind(hashed.as_str())
            .bind(&id.0)
            .execute(self.connection())
            .await
            .map_err(unavailable)?;

        Ok(Some(self.changed(before, id).await?))
    }

    async fn password_held_by(&mut self, id: &UserId) -> Result<Option<PasswordHash>, StoreError> {
        let found: Option<Option<String>> =
            sqlx::query_scalar("SELECT password_hash FROM users WHERE id = ?")
                .bind(&id.0)
                .fetch_optional(self.connection())
                .await
                .map_err(unavailable)?;

        Ok(found.flatten().map(PasswordHash))
    }

    async fn delete_user(&mut self, id: &UserId) -> Result<Option<Change>, AdministrationRefused> {
        let Some(before) = self.user(id).await? else {
            return Ok(None);
        };

        if is_the_last_administrator(self, id).await? {
            return Err(AdministrationRefused::LastSystemAdministrator);
        }

        sqlx::query("DELETE FROM users WHERE id = ?")
            .bind(&id.0)
            .execute(self.connection())
            .await
            .map_err(unavailable)?;

        Ok(Some(Change {
            before,
            // Nothing after it, which is the whole of what a deletion says.
            after: None,
        }))
    }

    async fn stored_password(
        &mut self,
        username: &str,
    ) -> Result<Option<StoredPassword>, StoreError> {
        // A locked account resolves to nothing, so it is refused exactly as a wrong password
        // is. Which half was wrong is not something an unauthenticated caller is entitled to.
        let found = sqlx::query(
            "SELECT id, password_hash FROM users WHERE username = ? AND password_hash IS NOT NULL \
             AND is_locked = 0",
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

impl Transaction {
    /// The change a write just made: the record as it stood, and as it stands now.
    ///
    /// The *after* is read back through the same transaction rather than assembled from what
    /// was asked for, so what the audit entry records is what the store holds.
    async fn changed(&mut self, before: User, id: &UserId) -> Result<Change, StoreError> {
        Ok(Change {
            before,
            after: self.user(id).await?,
        })
    }
}

/// A user, from the row every read of one selects.
fn a_user(row: &sqlx::sqlite::SqliteRow) -> User {
    User {
        id: UserId(row.get("id")),
        username: row.get("username"),
        is_system_administrator: row.get::<i64, _>("is_system_administrator") != 0,
        is_locked: row.get::<i64, _>("is_locked") != 0,
        has_password: row.get::<Option<String>, _>("password_hash").is_some(),
        external_identity: row
            .get::<Option<String>, _>("external_issuer")
            .map(|issuer| ExternalIdentity {
                issuer,
                subject: row.get("external_subject"),
            }),
    }
}

/// Whether this user is the only system administrator on the deployment.
///
/// It counts **flag holders and nothing else**. Narrowing it to the ones who could sign in
/// today — unlocked, with a password — reads as an improvement on the rule and is a hole in
/// it: an administrator who stops counting is one the next call may delete, so a box could
/// be emptied of administrators one permitted act at a time.
///
/// Recovering from a deployment nobody can sign into as an administrator is shell access to
/// the box, which is the highest privilege in the system and deliberately not something the
/// console can arrange ([ADR-0025]).
///
/// [ADR-0025]: ../../../docs/adr/0025-credentials-are-administered-because-there-is-no-email.md
async fn is_the_last_administrator(
    transaction: &mut Transaction,
    id: &UserId,
) -> Result<bool, StoreError> {
    // Two is enough to answer: either somebody else holds the flag, or this user is the only
    // one who does.
    let holders: Vec<String> =
        sqlx::query_scalar("SELECT id FROM users WHERE is_system_administrator = 1 LIMIT 2")
            .fetch_all(transaction.connection())
            .await
            .map_err(unavailable)?;

    Ok(holders.as_slice() == [id.0.clone()])
}

/// Tell a name that is already taken apart from a store that could not answer.
fn taken_or_unavailable(error: sqlx::Error, username: &str) -> AdministrationRefused {
    let taken = error
        .as_database_error()
        .is_some_and(sqlx::error::DatabaseError::is_unique_violation);

    if taken {
        AdministrationRefused::NameTaken {
            username: username.to_owned(),
        }
    } else {
        AdministrationRefused::Store(unavailable(error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::sign_ins::SignIns;
    use crate::configuration::store::a_temporary_store;

    async fn an_administrator(transaction: &mut Transaction, username: &str) -> UserId {
        transaction
            .create_user(NewUser {
                is_system_administrator: true,
                ..a_new_user(username)
            })
            .await
            .expect("an administrator")
    }

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
                is_locked: false,
                has_password: true,
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
            matches!(refusal, Err(AdministrationRefused::NameTaken { .. })),
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

    #[tokio::test]
    async fn locking_an_account_stops_the_name_resolving_to_a_credential() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let id = transaction
            .create_user(a_new_user("flight"))
            .await
            .expect("the user to be created");

        transaction
            .set_account_lock(&id, true)
            .await
            .expect("the lock to land");

        assert!(
            transaction
                .stored_password("flight")
                .await
                .expect("the read to answer")
                .is_none(),
            "a locked account still resolves to a credential"
        );
        assert!(
            transaction
                .user(&id)
                .await
                .expect("the read to answer")
                .expect("the user")
                .is_locked
        );
    }

    #[tokio::test]
    async fn locking_an_account_ends_every_sign_in_it_holds() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let id = transaction
            .create_user(a_new_user("flight"))
            .await
            .expect("the user to be created");
        let token = transaction
            .open_sign_in(&id)
            .await
            .expect("the sign-in to open");

        transaction
            .set_account_lock(&id, true)
            .await
            .expect("the lock to land");

        assert_eq!(
            transaction
                .holder_of(&token)
                .await
                .expect("the read to answer"),
            None,
            "a locked account is still signed in"
        );
    }

    /// The last system administrator cannot be removed (v1 §2), and *removed* means all
    /// three of the acts that would leave the deployment with nobody able to administer it.
    #[tokio::test]
    async fn the_last_system_administrator_cannot_be_locked_deleted_or_stripped_of_the_flag() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let last = an_administrator(&mut transaction, "root").await;

        let locked = transaction.set_account_lock(&last, true).await;
        let stripped = transaction.set_system_administration(&last, false).await;
        let deleted = transaction.delete_user(&last).await;

        for refusal in [locked, stripped, deleted] {
            assert!(
                matches!(refusal, Err(AdministrationRefused::LastSystemAdministrator)),
                "expected the last administrator to be kept, got {refusal:?}",
            );
        }
        assert!(
            transaction
                .user(&last)
                .await
                .expect("the read to answer")
                .expect("the administrator")
                .is_system_administrator
        );
    }

    #[tokio::test]
    async fn an_administrator_who_is_not_the_last_may_be_locked_stripped_and_deleted() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        an_administrator(&mut transaction, "root").await;
        let second = an_administrator(&mut transaction, "deputy").await;

        transaction
            .set_account_lock(&second, true)
            .await
            .expect("the lock to land");
        transaction
            .set_account_lock(&second, false)
            .await
            .expect("the unlock to land");
        transaction
            .set_system_administration(&second, false)
            .await
            .expect("the flag to be cleared");
        transaction
            .delete_user(&second)
            .await
            .expect("the deletion to land");

        assert_eq!(
            transaction.user(&second).await.expect("the read to answer"),
            None
        );
    }

    /// The rule counts flag holders and nothing else. Narrowing it to the ones who could
    /// sign in today would let a box be emptied of administrators one permitted act at a
    /// time: lock one, and the other stops being protected.
    #[tokio::test]
    async fn an_administrator_who_cannot_sign_in_still_counts_against_the_rule() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let locked_out = an_administrator(&mut transaction, "on-leave").await;
        let awaiting_enrolment = transaction
            .create_user(NewUser {
                username: "new-hire".to_owned(),
                password_hash: None,
                is_system_administrator: true,
            })
            .await
            .expect("an administrator with no password yet");
        transaction
            .set_account_lock(&locked_out, true)
            .await
            .expect("the lock to land");

        transaction
            .delete_user(&locked_out)
            .await
            .expect("one of three to go");

        let refusal = transaction.delete_user(&awaiting_enrolment).await;
        assert!(
            matches!(refusal, Err(AdministrationRefused::LastSystemAdministrator)),
            "expected the last flag holder to be kept, got {refusal:?}",
        );
    }

    /// A forced password reset is not one of the three acts the last administrator is
    /// protected from (v1 §2): the record and the flag both survive it, and the on-box CLI
    /// is what resets a password nobody left can reset.
    #[tokio::test]
    async fn forcing_a_password_reset_takes_the_password_and_leaves_the_record_standing() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let last = an_administrator(&mut transaction, "root").await;

        let change = transaction
            .clear_password(&last)
            .await
            .expect("the reset to land")
            .expect("a change");

        assert!(change.before.has_password);
        let after = change.after.expect("the record to survive");
        assert!(!after.has_password);
        assert!(after.is_system_administrator);
        assert!(
            transaction
                .stored_password("root")
                .await
                .expect("the read to answer")
                .is_none()
        );
    }

    #[tokio::test]
    async fn a_write_against_an_id_nobody_holds_changes_nothing_and_says_so() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let nobody = UserId::presented("no-such-id".to_owned());

        assert!(
            transaction
                .set_account_lock(&nobody, true)
                .await
                .expect("an answer")
                .is_none()
        );
        assert!(
            transaction
                .delete_user(&nobody)
                .await
                .expect("an answer")
                .is_none()
        );
        assert!(
            transaction
                .rename_user(&nobody, "renamed")
                .await
                .expect("an answer")
                .is_none()
        );
    }

    #[tokio::test]
    async fn setting_a_password_leaves_every_sign_in_standing() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let id = transaction
            .create_user(a_new_user("flight"))
            .await
            .expect("a user");
        let token = transaction.open_sign_in(&id).await.expect("a sign-in");

        let change = transaction
            .set_password(
                &id,
                PasswordHash::already_hashed("$argon2id$a-new-one".to_owned()),
            )
            .await
            .expect("the write to answer")
            .expect("a change");

        assert!(change.after.expect("a record after it").has_password);
        assert_eq!(
            transaction
                .password_held_by(&id)
                .await
                .expect("the read to answer")
                .expect("a password"),
            PasswordHash::already_hashed("$argon2id$a-new-one".to_owned())
        );
        assert_eq!(
            transaction
                .holder_of(&token)
                .await
                .expect("the read to answer"),
            Some(id),
            "setting a password ended a sign-in"
        );
    }

    /// Which is what makes re-presenting the current password impossible rather than
    /// skippable for an account awaiting enrolment.
    #[tokio::test]
    async fn a_user_awaiting_enrolment_holds_no_password_to_re_present() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let id = transaction
            .create_user(NewUser {
                password_hash: None,
                ..a_new_user("flight")
            })
            .await
            .expect("a user");

        assert_eq!(
            transaction
                .password_held_by(&id)
                .await
                .expect("the read to answer"),
            None
        );
        assert_eq!(
            transaction
                .password_held_by(&UserId::presented("no-such-id".to_owned()))
                .await
                .expect("the read to answer"),
            None
        );
    }

    #[tokio::test]
    async fn a_user_is_read_back_by_the_name_a_human_types_however_it_was_capitalised() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let id = transaction
            .create_user(a_new_user("flight"))
            .await
            .expect("a user");

        let found = transaction
            .user_named("FLIGHT")
            .await
            .expect("the read to answer")
            .expect("a user");

        assert_eq!(found.id, id);
        assert_eq!(
            transaction
                .user_named("nobody")
                .await
                .expect("the read to answer"),
            None
        );
    }
}
