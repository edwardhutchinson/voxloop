//! Eligibility: who may assume which role.
//!
//! An eligibility is the **unconditional grant permitting a user to take up a role**, and it
//! carries no permissions of its own (v1 §1). It says *this person may sit in that seat* and
//! stops there: what the seat can hear, say or command is one cell on the grid, and there is
//! nothing here that widens one. Reach comes from the grid or from nowhere.
//!
//! It is deliberately **not a second grid**. There is no value, no rung and no ordering —
//! the pair is granted or it is not — and, more importantly, there is no read of the whole.
//! Rendered as a matrix, 190 users by 15 roles was the least legible object the console
//! prototype produced ([ADR-0015]), so eligibility is administered from **two directions**
//! and there is no third: [`Eligibilities::the_users_eligible_for`] is the role page,
//! answering *who may assume this*, and [`Eligibilities::the_roles_open_to`] is the user
//! page, answering *which roles may this person assume*. A method answering the whole of it
//! would be the wall, and it is absent on purpose rather than merely unbuilt.
//!
//! **Every user starts eligible for `Observer`** (v1 §2), seeded as part of creating the
//! record rather than by whoever happens to create one — the console, the on-box CLI and the
//! bootstrap route all make users, and a rule three callers have to remember is a rule with
//! three chances of being forgotten.
//!
//! Revoking eligibility from somebody occupying the role **ends their occupancy immediately,
//! with the reason shown** (v1 §2's lifetime table). That half is live state and arrives
//! with sessions (#25); what is here is the configuration write it will hang off.
//!
//! [ADR-0015]: ../../../docs/adr/0015-the-admin-console-reads-one-row-at-a-time.md

use async_trait::async_trait;

use super::records::Change;
use super::roles::{Role, RoleId, Roles, a_role};
use super::store::{StoreError, Transaction, now, unavailable};
use super::users::{User, UserId, Users, a_user};

/// One eligibility: that this user may assume this role.
///
/// It carries both records rather than their ids, for the reason a grid cell does — every
/// reader wants the names. The console renders them and the audit log snapshots them, and a
/// grant read without them is a pair of opaque strings nobody can act on.
///
/// There is no third field, and the absence is the model: an eligibility has no rung, no
/// condition and no expiry. It exists or it does not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Eligibility {
    pub(crate) user: User,
    pub(crate) for_role: Role,
}

/// Eligibility, as domain operations rather than queries ([ADR-0038]).
///
/// [ADR-0038]: ../../../docs/adr/0038-sqlite-behind-domain-shaped-repositories.md
#[async_trait]
pub(crate) trait Eligibilities {
    /// Whether this user may assume this role: **one lookup**, and the whole of what
    /// eligibility answers.
    ///
    /// It is the read *assume* is gated on, and it says nothing about reach — a user
    /// eligible for a role with an empty row may assume it and reach nothing, which is an
    /// ordinary configuration rather than a contradiction.
    async fn is_eligible(&mut self, user: &UserId, for_role: &RoleId) -> Result<bool, StoreError>;

    /// Grant eligibility, answering with the grant as it stands.
    ///
    /// Granting what is already granted is the same grant rather than a second one, so it
    /// leaves the original grant time alone and answers with what stands. The act is still
    /// audited, because an administrator who pressed it made a decision whether or not the
    /// store had to change.
    ///
    /// A pair naming a user or a role that is not there is no change rather than a refusal,
    /// exactly as a write against any other id nobody holds is.
    async fn grant_eligibility(
        &mut self,
        user: &UserId,
        for_role: &RoleId,
    ) -> Result<Option<Eligibility>, StoreError>;

    /// One eligibility as it stands, or nothing where the pair names no grant.
    ///
    /// It reports rather than decides. [`Eligibilities::is_eligible`] is the lookup every
    /// decision is made from — one column, one answer — and this is the same fact carrying
    /// the two records, for a console to render and an audit entry to snapshot.
    async fn an_eligibility(
        &mut self,
        user: &UserId,
        for_role: &RoleId,
    ) -> Result<Option<Eligibility>, StoreError>;

    /// Revoke eligibility, answering with what was revoked.
    ///
    /// Nothing where the pair names no grant — revoking what nobody holds is a not-found
    /// rather than a quiet success, so an administrator working from a stale page is told
    /// their page is stale instead of being shown a revocation that never happened.
    ///
    /// The occupancy this ends is live state and lands with sessions (#25). Here it is a
    /// configuration write and nothing else.
    async fn revoke_eligibility(
        &mut self,
        user: &UserId,
        for_role: &RoleId,
    ) -> Result<Option<Change<Eligibility>>, StoreError>;

    /// A role page's half: the role, and everyone who may assume it, by name.
    ///
    /// The eligible **and nobody else**. A role page listing every user on the deployment
    /// with a mark against some of them is a column of the matrix ADR-0015 rejected, and at
    /// 190 users it is the same wall one slice at a time. Who is not eligible is read from
    /// the user list, which is a different page answering a different question.
    async fn the_users_eligible_for(
        &mut self,
        for_role: &RoleId,
    ) -> Result<Option<(Role, Vec<User>)>, StoreError>;

    /// A user page's half: the user, and every role they may assume, by name.
    ///
    /// The same rule in the other direction, for the same reason.
    async fn the_roles_open_to(
        &mut self,
        user: &UserId,
    ) -> Result<Option<(User, Vec<Role>)>, StoreError>;
}

#[async_trait]
impl Eligibilities for Transaction {
    async fn is_eligible(&mut self, user: &UserId, for_role: &RoleId) -> Result<bool, StoreError> {
        let found: Option<i64> =
            sqlx::query_scalar("SELECT 1 FROM eligibility WHERE user_id = ? AND role_id = ?")
                .bind(user.as_str())
                .bind(for_role.as_str())
                .fetch_optional(self.connection())
                .await
                .map_err(unavailable)?;

        Ok(found.is_some())
    }

    async fn grant_eligibility(
        &mut self,
        user: &UserId,
        for_role: &RoleId,
    ) -> Result<Option<Eligibility>, StoreError> {
        // Both records are read first, so a pair naming something nobody holds writes
        // nothing — the foreign keys would refuse it anyway, and a refusal from the store is
        // not an answer this seam is allowed to hand back ([ADR-0060]).
        if self.the_pair(user, for_role).await?.is_none() {
            return Ok(None);
        }

        sqlx::query(
            "INSERT INTO eligibility (user_id, role_id, granted_at) VALUES (?, ?, ?) \
             ON CONFLICT (user_id, role_id) DO NOTHING",
        )
        .bind(user.as_str())
        .bind(for_role.as_str())
        .bind(now())
        .execute(self.connection())
        .await
        .map_err(unavailable)?;

        // Read back through the same transaction, so what the caller audits is what the
        // store holds rather than what it was asked for.
        self.an_eligibility(user, for_role).await
    }

    async fn an_eligibility(
        &mut self,
        user: &UserId,
        for_role: &RoleId,
    ) -> Result<Option<Eligibility>, StoreError> {
        if !self.is_eligible(user, for_role).await? {
            return Ok(None);
        }

        self.the_pair(user, for_role).await
    }

    async fn revoke_eligibility(
        &mut self,
        user: &UserId,
        for_role: &RoleId,
    ) -> Result<Option<Change<Eligibility>>, StoreError> {
        let Some(before) = self.an_eligibility(user, for_role).await? else {
            return Ok(None);
        };

        sqlx::query("DELETE FROM eligibility WHERE user_id = ? AND role_id = ?")
            .bind(user.as_str())
            .bind(for_role.as_str())
            .execute(self.connection())
            .await
            .map_err(unavailable)?;

        Ok(Some(Change {
            before,
            // Nothing after it. A revoked eligibility is not a grant holding some lesser
            // value; it is gone, which is the whole of what revoking one says.
            after: None,
        }))
    }

    async fn the_users_eligible_for(
        &mut self,
        for_role: &RoleId,
    ) -> Result<Option<(Role, Vec<User>)>, StoreError> {
        let Some(role) = self.role(for_role).await? else {
            return Ok(None);
        };

        let rows = sqlx::query(
            "SELECT users.id AS id, users.username AS username, \
                    users.is_system_administrator AS is_system_administrator, \
                    users.is_locked AS is_locked, users.password_hash AS password_hash, \
                    users.external_issuer AS external_issuer, \
                    users.external_subject AS external_subject \
             FROM eligibility \
             JOIN users ON users.id = eligibility.user_id \
             WHERE eligibility.role_id = ? \
             ORDER BY users.username",
        )
        .bind(role.id.as_str())
        .fetch_all(self.connection())
        .await
        .map_err(unavailable)?;

        Ok(Some((role, rows.iter().map(a_user).collect())))
    }

    async fn the_roles_open_to(
        &mut self,
        user: &UserId,
    ) -> Result<Option<(User, Vec<Role>)>, StoreError> {
        let Some(user) = self.user(user).await? else {
            return Ok(None);
        };

        let rows = sqlx::query(
            "SELECT roles.id AS id, roles.name AS name, roles.max_occupants AS max_occupants \
             FROM eligibility \
             JOIN roles ON roles.id = eligibility.role_id \
             WHERE eligibility.user_id = ? \
             ORDER BY roles.name",
        )
        .bind(user.id.as_str())
        .fetch_all(self.connection())
        .await
        .map_err(unavailable)?;

        Ok(Some((user, rows.iter().map(a_role).collect())))
    }
}

impl Transaction {
    /// The two records the pair names, whether or not the grant between them is held.
    ///
    /// It is what every write here needs, and what makes *no such user or role* answerable
    /// without a second round trip.
    async fn the_pair(
        &mut self,
        user: &UserId,
        for_role: &RoleId,
    ) -> Result<Option<Eligibility>, StoreError> {
        let (Some(user), Some(for_role)) = (self.user(user).await?, self.role(for_role).await?)
        else {
            return Ok(None);
        };

        Ok(Some(Eligibility { user, for_role }))
    }

    /// Seed a new user's `Observer` eligibility, where the deployment still has an
    /// `Observer`.
    ///
    /// Every user record starts with it (v1 §2), so this is part of creating one rather than
    /// a step each of the three callers that create users has to remember.
    ///
    /// The role is found **by name**: the seeded `Observer` carries an id minted at install
    /// and nothing marks it as the seeded one. A deployment that renamed or deleted it has
    /// decided what its listen-only position is, and VoxLoop guessing which role replaced it
    /// would be worse than seeding nothing — so nothing is what it seeds. That is the same
    /// answer install gives when it seeds `Observer`'s reach against the loops present at
    /// the time and there are none.
    pub(super) async fn seed_observer_eligibility(
        &mut self,
        user: &UserId,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO eligibility (user_id, role_id, granted_at) \
             SELECT ?, roles.id, ? FROM roles WHERE roles.name = 'Observer'",
        )
        .bind(user.as_str())
        .bind(now())
        .execute(self.connection())
        .await
        .map_err(unavailable)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::roles::NewRole;
    use crate::configuration::store::a_temporary_store;
    use crate::configuration::users::NewUser;

    async fn a_user_named(transaction: &mut Transaction, username: &str) -> UserId {
        transaction
            .create_user(NewUser {
                username: username.to_owned(),
                password_hash: None,
                is_system_administrator: false,
            })
            .await
            .expect("the user to be created")
    }

    async fn a_role_named(transaction: &mut Transaction, name: &str) -> RoleId {
        transaction
            .create_role(NewRole {
                name: name.to_owned(),
                max_occupants: Some(1),
            })
            .await
            .expect("the role to be created")
    }

    /// The role every deployment is installed with, by the name install gives it.
    async fn the_observer_role(transaction: &mut Transaction) -> RoleId {
        transaction
            .roles()
            .await
            .expect("the roles to be read")
            .into_iter()
            .find(|role| role.name == "Observer")
            .expect("the seeded Observer role")
            .id
    }

    #[tokio::test]
    async fn nobody_is_eligible_for_a_role_nobody_granted_them() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = a_user_named(&mut transaction, "flight").await;
        let for_role = a_role_named(&mut transaction, "Flight Director").await;

        assert!(
            !transaction
                .is_eligible(&user, &for_role)
                .await
                .expect("the lookup to answer")
        );
    }

    #[tokio::test]
    async fn grants_eligibility_and_answers_with_both_records() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = a_user_named(&mut transaction, "flight").await;
        let for_role = a_role_named(&mut transaction, "Flight Director").await;

        let granted = transaction
            .grant_eligibility(&user, &for_role)
            .await
            .expect("the grant to be made")
            .expect("a grant");

        assert_eq!(granted.user.username, "flight");
        assert_eq!(granted.for_role.name, "Flight Director");
        assert!(
            transaction
                .is_eligible(&user, &for_role)
                .await
                .expect("the lookup to answer")
        );
    }

    /// Granting twice is the same grant, not two of them, and it leaves the first alone.
    #[tokio::test]
    async fn granting_what_is_already_granted_changes_nothing() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = a_user_named(&mut transaction, "flight").await;
        let for_role = a_role_named(&mut transaction, "Flight Director").await;
        transaction
            .grant_eligibility(&user, &for_role)
            .await
            .expect("the first grant to be made");

        transaction
            .grant_eligibility(&user, &for_role)
            .await
            .expect("the second grant to be made")
            .expect("a grant");

        let (_, eligible) = transaction
            .the_users_eligible_for(&for_role)
            .await
            .expect("the read to answer")
            .expect("the role");
        assert_eq!(eligible.len(), 1, "the grant was made twice: {eligible:?}");
    }

    #[tokio::test]
    async fn revokes_eligibility_and_answers_with_what_was_revoked() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = a_user_named(&mut transaction, "flight").await;
        let for_role = a_role_named(&mut transaction, "Flight Director").await;
        transaction
            .grant_eligibility(&user, &for_role)
            .await
            .expect("the grant to be made");

        let change = transaction
            .revoke_eligibility(&user, &for_role)
            .await
            .expect("the revocation to be made")
            .expect("a change");

        assert_eq!(change.before.user.username, "flight");
        assert_eq!(change.before.for_role.name, "Flight Director");
        assert_eq!(change.after, None);
        assert!(
            !transaction
                .is_eligible(&user, &for_role)
                .await
                .expect("the lookup to answer")
        );
    }

    /// Revoking what nobody holds is a not-found rather than a quiet success, so an
    /// administrator working from a stale page is told their page is stale.
    #[tokio::test]
    async fn revoking_what_was_never_granted_is_no_change() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = a_user_named(&mut transaction, "flight").await;
        let for_role = a_role_named(&mut transaction, "Flight Director").await;

        assert_eq!(
            transaction
                .revoke_eligibility(&user, &for_role)
                .await
                .expect("the revocation to answer"),
            None
        );
    }

    #[tokio::test]
    async fn a_grant_against_a_user_or_a_role_nobody_holds_is_no_change() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = a_user_named(&mut transaction, "flight").await;
        let for_role = a_role_named(&mut transaction, "Flight Director").await;

        assert!(
            transaction
                .grant_eligibility(&UserId::presented("nobody".to_owned()), &for_role)
                .await
                .expect("the grant to answer")
                .is_none()
        );
        assert!(
            transaction
                .grant_eligibility(&user, &RoleId::presented("nothing".to_owned()))
                .await
                .expect("the grant to answer")
                .is_none()
        );
    }

    /// The role page's question: *who may assume this*.
    #[tokio::test]
    async fn a_role_answers_who_may_assume_it_and_nobody_else() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let eligible = a_user_named(&mut transaction, "capcom").await;
        a_user_named(&mut transaction, "booster").await;
        let for_role = a_role_named(&mut transaction, "Flight Director").await;
        transaction
            .grant_eligibility(&eligible, &for_role)
            .await
            .expect("the grant to be made");

        let (role, may_assume) = transaction
            .the_users_eligible_for(&for_role)
            .await
            .expect("the read to answer")
            .expect("the role");

        assert_eq!(role.name, "Flight Director");
        let names: Vec<&str> = may_assume
            .iter()
            .map(|user| user.username.as_str())
            .collect();
        assert_eq!(names, ["capcom"]);
        assert!(
            !names.contains(&"booster"),
            "the ineligible user was listed: {names:?}"
        );
    }

    /// The user page's question: *which roles may this person assume*.
    #[tokio::test]
    async fn a_user_answers_which_roles_they_may_assume_and_no_others() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = a_user_named(&mut transaction, "capcom").await;
        let granted = a_role_named(&mut transaction, "Flight Director").await;
        a_role_named(&mut transaction, "Thermal Engineer").await;
        transaction
            .grant_eligibility(&user, &granted)
            .await
            .expect("the grant to be made");

        let (read, open) = transaction
            .the_roles_open_to(&user)
            .await
            .expect("the read to answer")
            .expect("the user");

        assert_eq!(read.username, "capcom");
        let names: Vec<&str> = open.iter().map(|role| role.name.as_str()).collect();
        // `Observer` is seeded on creation, and `Thermal Engineer` was never granted.
        assert_eq!(names, ["Flight Director", "Observer"]);
    }

    #[tokio::test]
    async fn a_user_or_a_role_nobody_holds_has_no_page_to_read() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");

        assert!(
            transaction
                .the_users_eligible_for(&RoleId::presented("nothing".to_owned()))
                .await
                .expect("the read to answer")
                .is_none()
        );
        assert!(
            transaction
                .the_roles_open_to(&UserId::presented("nobody".to_owned()))
                .await
                .expect("the read to answer")
                .is_none()
        );
    }

    /// Every user record starts with seeded `Observer` eligibility (v1 §2), whichever of the
    /// three callers that create users made it.
    #[tokio::test]
    async fn creating_a_user_seeds_observer_eligibility() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let observer = the_observer_role(&mut transaction).await;

        let user = a_user_named(&mut transaction, "flight").await;

        assert!(
            transaction
                .is_eligible(&user, &observer)
                .await
                .expect("the lookup to answer"),
            "a new user was not seeded with Observer eligibility"
        );
    }

    /// A deployment that deleted `Observer` has decided what its listen-only position is,
    /// and VoxLoop guessing which role replaced it would be worse than seeding nothing.
    #[tokio::test]
    async fn a_deployment_with_no_observer_role_seeds_nothing() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let observer = the_observer_role(&mut transaction).await;
        transaction
            .delete_role(&observer)
            .await
            .expect("the role to be deleted");

        let user = a_user_named(&mut transaction, "flight").await;

        let (_, open) = transaction
            .the_roles_open_to(&user)
            .await
            .expect("the read to answer")
            .expect("the user");
        assert!(open.is_empty(), "something was seeded: {open:?}");
    }

    /// Eligibility carries no permissions of its own, so deleting the record it is about
    /// takes the grant with it rather than leaving one nobody could exercise.
    #[tokio::test]
    async fn deleting_a_role_takes_the_eligibility_for_it_with_it() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = a_user_named(&mut transaction, "flight").await;
        let for_role = a_role_named(&mut transaction, "Flight Director").await;
        transaction
            .grant_eligibility(&user, &for_role)
            .await
            .expect("the grant to be made");

        transaction
            .delete_role(&for_role)
            .await
            .expect("the role to be deleted");

        assert!(
            !transaction
                .is_eligible(&user, &for_role)
                .await
                .expect("the lookup to answer")
        );
    }

    #[tokio::test]
    async fn deleting_a_user_takes_their_eligibility_with_them() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = a_user_named(&mut transaction, "flight").await;
        let for_role = a_role_named(&mut transaction, "Flight Director").await;
        transaction
            .grant_eligibility(&user, &for_role)
            .await
            .expect("the grant to be made");

        transaction
            .delete_user(&user)
            .await
            .expect("the user to be deleted");

        let (_, eligible) = transaction
            .the_users_eligible_for(&for_role)
            .await
            .expect("the read to answer")
            .expect("the role");
        assert!(
            eligible.is_empty(),
            "somebody was left eligible: {eligible:?}"
        );
    }

    /// A rename changes nothing that references the record, here as everywhere: everything
    /// holds the immutable id, so a grant survives both halves of its pair being renamed.
    #[tokio::test]
    async fn a_grant_survives_a_rename_of_either_record() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = a_user_named(&mut transaction, "flight").await;
        let for_role = a_role_named(&mut transaction, "Flight Director").await;
        transaction
            .grant_eligibility(&user, &for_role)
            .await
            .expect("the grant to be made");

        transaction
            .rename_user(&user, "flight-director")
            .await
            .expect("the user to be renamed");
        transaction
            .rename_role(&for_role, "Flight")
            .await
            .expect("the role to be renamed");

        assert!(
            transaction
                .is_eligible(&user, &for_role)
                .await
                .expect("the lookup to answer")
        );
    }
}
