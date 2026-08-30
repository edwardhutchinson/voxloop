//! The role record, and the repository that holds it.
//!
//! A **role is a staffable position** with a `max_occupants` limit, not a group of users (v1
//! §1). Single-occupant and multi-occupant roles are the same concept under different
//! limits, so there is one record here and no kinds.
//!
//! Everything else references the immutable id, so renaming a role breaks nothing — and that
//! is a property to be tested rather than intended, since a stray join on a role name works
//! perfectly until the first rename.

use async_trait::async_trait;
use sqlx::Row;

use super::records::{AdministrationRefused, Change, taken_or_unavailable};
use super::store::{StoreError, Transaction, now, unavailable};
use crate::secrets;

/// The immutable opaque internal id of a role, never reused.
///
/// The grid, eligibility, service principals and every audit entry hold this and never the
/// name, which is what makes a rename safe.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct RoleId(String);

impl RoleId {
    /// Take back an id the store already minted.
    fn known(id: String) -> Self {
        Self(id)
    }

    /// Take an id as a caller presented it, in a path or a request body.
    ///
    /// It is opaque to everything outside this module, so there is nothing to validate here:
    /// an id nobody holds reads back as no role, which is the answer the caller needed.
    pub(crate) fn presented(id: String) -> Self {
        Self(id)
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// A role, as everything outside this module sees one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Role {
    pub(crate) id: RoleId,
    pub(crate) name: String,
    /// How many users may occupy this role at once, where there is a limit.
    ///
    /// `None` is *no limit*: the same concept with the limit left unset, rather than a second
    /// kind of role. `Observer` is seeded that way, because every user is eligible for it and
    /// any number VoxLoop picked instead would be a guess discovered only by the person it
    /// turned away.
    pub(crate) max_occupants: Option<u32>,
}

/// A role about to exist.
pub(crate) struct NewRole {
    pub(crate) name: String,
    pub(crate) max_occupants: Option<u32>,
}

/// The role record, as domain operations rather than queries ([ADR-0038]).
///
/// [ADR-0038]: ../../../docs/adr/0038-sqlite-behind-domain-shaped-repositories.md
#[async_trait]
pub(crate) trait Roles {
    /// Create a role, and answer with the id nothing will ever change.
    async fn create_role(&mut self, new: NewRole) -> Result<RoleId, AdministrationRefused>;

    /// Read a role by the id that identifies it.
    async fn role(&mut self, id: &RoleId) -> Result<Option<Role>, StoreError>;

    /// Every role on the deployment, by name.
    ///
    /// Alphabetically, deliberately: a role list is picked from rather than read down, and
    /// the administered ordering VoxLoop holds is the **loop** order, which is a different
    /// thing for a different reason ([ADR-0053]).
    ///
    /// [ADR-0053]: ../../../docs/adr/0053-the-loop-order-is-complete-and-a-new-loop-lands-at-the-end.md
    async fn roles(&mut self) -> Result<Vec<Role>, StoreError>;

    /// Change the name a role is known by, leaving everything that references it alone.
    async fn rename_role(
        &mut self,
        id: &RoleId,
        name: &str,
    ) -> Result<Option<Change<Role>>, AdministrationRefused>;

    /// Set how many users may occupy this role at once, or take the limit away.
    async fn set_max_occupants(
        &mut self,
        id: &RoleId,
        max_occupants: Option<u32>,
    ) -> Result<Option<Change<Role>>, AdministrationRefused>;

    /// Delete a role, and everything that is only about it.
    ///
    /// Its audit entries stay, readable and attributed by the name as it stood ([ADR-0028]).
    ///
    /// [ADR-0028]: ../../../docs/adr/0028-the-audit-log-records-decisions-not-traffic.md
    async fn delete_role(&mut self, id: &RoleId) -> Result<Option<Change<Role>>, StoreError>;
}

#[async_trait]
impl Roles for Transaction {
    async fn create_role(&mut self, new: NewRole) -> Result<RoleId, AdministrationRefused> {
        refuse_an_empty_role(new.max_occupants)?;
        let id = RoleId(secrets::unguessable());

        sqlx::query("INSERT INTO roles (id, name, max_occupants, created_at) VALUES (?, ?, ?, ?)")
            .bind(&id.0)
            .bind(&new.name)
            .bind(new.max_occupants)
            .bind(now())
            .execute(self.connection())
            .await
            .map_err(|error| taken_or_unavailable(error, "role name", &new.name))?;

        Ok(id)
    }

    async fn role(&mut self, id: &RoleId) -> Result<Option<Role>, StoreError> {
        let found = sqlx::query("SELECT id, name, max_occupants FROM roles WHERE id = ?")
            .bind(&id.0)
            .fetch_optional(self.connection())
            .await
            .map_err(unavailable)?;

        Ok(found.as_ref().map(a_role))
    }

    async fn roles(&mut self) -> Result<Vec<Role>, StoreError> {
        let rows = sqlx::query("SELECT id, name, max_occupants FROM roles ORDER BY name")
            .fetch_all(self.connection())
            .await
            .map_err(unavailable)?;

        Ok(rows.iter().map(a_role).collect())
    }

    async fn rename_role(
        &mut self,
        id: &RoleId,
        name: &str,
    ) -> Result<Option<Change<Role>>, AdministrationRefused> {
        let Some(before) = self.role(id).await? else {
            return Ok(None);
        };

        sqlx::query("UPDATE roles SET name = ? WHERE id = ?")
            .bind(name)
            .bind(&id.0)
            .execute(self.connection())
            .await
            .map_err(|error| taken_or_unavailable(error, "role name", name))?;

        Ok(Some(self.role_changed(before, id).await?))
    }

    async fn set_max_occupants(
        &mut self,
        id: &RoleId,
        max_occupants: Option<u32>,
    ) -> Result<Option<Change<Role>>, AdministrationRefused> {
        refuse_an_empty_role(max_occupants)?;

        let Some(before) = self.role(id).await? else {
            return Ok(None);
        };

        sqlx::query("UPDATE roles SET max_occupants = ? WHERE id = ?")
            .bind(max_occupants)
            .bind(&id.0)
            .execute(self.connection())
            .await
            .map_err(unavailable)?;

        Ok(Some(self.role_changed(before, id).await?))
    }

    async fn delete_role(&mut self, id: &RoleId) -> Result<Option<Change<Role>>, StoreError> {
        let Some(before) = self.role(id).await? else {
            return Ok(None);
        };

        sqlx::query("DELETE FROM roles WHERE id = ?")
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
}

impl Transaction {
    /// The change a write just made: the record as it stood, and as it stands now.
    ///
    /// The *after* is read back through the same transaction rather than assembled from what
    /// was asked for, so what the audit entry records is what the store holds.
    async fn role_changed(
        &mut self,
        before: Role,
        id: &RoleId,
    ) -> Result<Change<Role>, StoreError> {
        Ok(Change {
            before,
            after: self.role(id).await?,
        })
    }
}

/// A role, from the row every read of one selects.
fn a_role(row: &sqlx::sqlite::SqliteRow) -> Role {
    Role {
        id: RoleId::known(row.get("id")),
        name: row.get("name"),
        max_occupants: row
            .get::<Option<i64>, _>("max_occupants")
            .and_then(|limit| u32::try_from(limit).ok()),
    }
}

/// A role nobody may occupy is not a staffable position, so it is refused rather than stored.
fn refuse_an_empty_role(max_occupants: Option<u32>) -> Result<(), AdministrationRefused> {
    match max_occupants {
        Some(0) => Err(AdministrationRefused::NobodyMayOccupy),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::store::a_temporary_store;

    fn a_new_role(name: &str) -> NewRole {
        NewRole {
            name: name.to_owned(),
            max_occupants: Some(1),
        }
    }

    #[tokio::test]
    async fn install_seeds_the_observer_role_with_no_limit_on_how_many_may_observe() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");

        let roles = transaction.roles().await.expect("the read to answer");

        let observer = roles
            .iter()
            .find(|role| role.name == "Observer")
            .expect("the seeded Observer role");
        assert_eq!(observer.max_occupants, None);
        assert_eq!(
            roles.len(),
            1,
            "install seeded more than Observer: {roles:?}"
        );
    }

    #[tokio::test]
    async fn creates_a_role_and_reads_it_back_by_the_id_it_minted() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");

        let id = transaction
            .create_role(NewRole {
                name: "Flight Director".to_owned(),
                max_occupants: Some(1),
            })
            .await
            .expect("the role to be created");

        assert_eq!(
            transaction.role(&id).await.expect("the read to answer"),
            Some(Role {
                id,
                name: "Flight Director".to_owned(),
                max_occupants: Some(1),
            })
        );
    }

    #[tokio::test]
    async fn a_multi_occupant_role_is_the_same_record_under_a_different_limit() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");

        let single = transaction
            .create_role(a_new_role("Flight Director"))
            .await
            .expect("the single-occupant role");
        let many = transaction
            .create_role(NewRole {
                name: "Support Engineer".to_owned(),
                max_occupants: Some(6),
            })
            .await
            .expect("the multi-occupant role");

        let occupancy = |role: Option<Role>| role.expect("the role").max_occupants;
        assert_eq!(
            occupancy(transaction.role(&single).await.expect("a read")),
            Some(1)
        );
        assert_eq!(
            occupancy(transaction.role(&many).await.expect("a read")),
            Some(6)
        );
    }

    #[tokio::test]
    async fn refuses_a_role_name_already_taken_whatever_its_case() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        transaction
            .create_role(a_new_role("Flight Director"))
            .await
            .expect("the first role");

        let refusal = transaction.create_role(a_new_role("FLIGHT DIRECTOR")).await;

        assert!(
            matches!(refusal, Err(AdministrationRefused::NameTaken { .. })),
            "expected the name to be refused, got {refusal:?}"
        );
    }

    #[tokio::test]
    async fn refuses_a_role_nobody_may_occupy() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");

        let refusal = transaction
            .create_role(NewRole {
                name: "Nobody".to_owned(),
                max_occupants: Some(0),
            })
            .await;

        assert!(
            matches!(refusal, Err(AdministrationRefused::NobodyMayOccupy)),
            "expected a role nobody may occupy to be refused, got {refusal:?}"
        );
    }

    #[tokio::test]
    async fn renaming_a_role_leaves_the_id_everything_references_alone() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let id = transaction
            .create_role(a_new_role("Flight Director"))
            .await
            .expect("the role to be created");

        let change = transaction
            .rename_role(&id, "Flight")
            .await
            .expect("the rename to land")
            .expect("a change");

        assert_eq!(change.before.name, "Flight Director");
        assert_eq!(change.after.expect("the role after").name, "Flight");
        assert_eq!(
            transaction
                .role(&id)
                .await
                .expect("the read to answer")
                .expect("the role")
                .name,
            "Flight",
            "the id no longer reads back the role it named"
        );
    }

    #[tokio::test]
    async fn setting_the_limit_answers_with_the_record_before_and_after() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let id = transaction
            .create_role(a_new_role("Support Engineer"))
            .await
            .expect("the role to be created");

        let change = transaction
            .set_max_occupants(&id, None)
            .await
            .expect("the limit to be set")
            .expect("a change");

        assert_eq!(change.before.max_occupants, Some(1));
        assert_eq!(change.after.expect("the role after").max_occupants, None);
    }

    #[tokio::test]
    async fn deleting_a_role_says_what_was_there_and_nothing_after_it() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let id = transaction
            .create_role(a_new_role("Flight Director"))
            .await
            .expect("the role to be created");

        let change = transaction
            .delete_role(&id)
            .await
            .expect("the deletion to land")
            .expect("a change");

        assert_eq!(change.before.name, "Flight Director");
        assert!(change.after.is_none(), "a deletion left something behind");
        assert!(
            transaction
                .role(&id)
                .await
                .expect("the read to answer")
                .is_none()
        );
    }

    #[tokio::test]
    async fn a_write_against_an_id_nobody_holds_is_no_change_rather_than_a_refusal() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let nobody = RoleId::presented("no-such-role".to_owned());

        assert!(
            transaction
                .rename_role(&nobody, "Flight")
                .await
                .expect("the write to answer")
                .is_none()
        );
        assert!(
            transaction
                .delete_role(&nobody)
                .await
                .expect("the write to answer")
                .is_none()
        );
    }
}
