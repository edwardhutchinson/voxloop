//! The loop record, the repository that holds it, and the deployment-wide base loop order.
//!
//! A **loop is an audio conference** and the only thing voice can be addressed to
//! ([ADR-0001]). There is no loop kind, no conference loop, no breakout mechanism and no
//! naming convention: a private room is an ordinary loop an administrator configured, and
//! VoxLoop neither knows nor cares ([ADR-0055]).
//!
//! Two things about a loop are decided here rather than by whoever creates one:
//!
//! - **A loop arrives `unreviewed`** and stays that way until an administrator has set or
//!   explicitly dismissed each role's cell ([ADR-0015]). Dismissing it is per loop and
//!   records deliberate `none`s, so it arrives with the grid; until then a loop that exists
//!   says plainly that nobody has ruled on it. It is a display state throughout: the
//!   evaluator enforces an unreviewed loop's cells as `none` like any other absent cell.
//! - **The base order is administered, not derived** ([ADR-0053]) — not alphabetical, and
//!   not creation order. A site that runs `FLIGHT`, `GNC`, `THERMAL` on every wall display
//!   wants that order on the console too. A new loop lands at the end, because appending is
//!   the only honest placement for something VoxLoop has been told nothing about.
//!
//! [ADR-0001]: ../../../docs/adr/0001-the-loop-is-the-only-destination.md
//! [ADR-0015]: ../../../docs/adr/0015-the-admin-console-reads-one-row-at-a-time.md
//! [ADR-0053]: ../../../docs/adr/0053-the-loop-order-is-complete-and-a-new-loop-lands-at-the-end.md
//! [ADR-0055]: ../../../docs/adr/0055-there-is-no-conference-loop.md

use async_trait::async_trait;
use sqlx::Row;

use super::records::{AdministrationRefused, Change, taken_or_unavailable};
use super::store::{StoreError, Transaction, now, unavailable};
use crate::secrets;

/// The immutable opaque internal id of a loop, never reused.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct LoopId(String);

impl LoopId {
    /// Take back an id the store already minted.
    fn known(id: String) -> Self {
        Self(id)
    }

    /// Take an id as a caller presented it, in a path or a request body.
    pub(crate) fn presented(id: String) -> Self {
        Self(id)
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// A loop, as everything outside this module sees one.
///
/// There is no kind, type or category here, deliberately ([ADR-0055]).
///
/// [ADR-0055]: ../../../docs/adr/0055-there-is-no-conference-loop.md
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Loop {
    pub(crate) id: LoopId,
    pub(crate) name: String,
    /// Whether an administrator has yet ruled on this loop's column ([ADR-0015]).
    ///
    /// [ADR-0015]: ../../../docs/adr/0015-the-admin-console-reads-one-row-at-a-time.md
    pub(crate) is_unreviewed: bool,
}

/// The loop record and the order the deployment reads them in, as domain operations rather
/// than queries ([ADR-0038]).
///
/// [ADR-0038]: ../../../docs/adr/0038-sqlite-behind-domain-shaped-repositories.md
#[async_trait]
pub(crate) trait Loops {
    /// Create a loop at the end of the base order, unreviewed.
    ///
    /// Both are properties of the act rather than choices a caller makes: a loop created
    /// after install is unreviewed (v1 §9), and VoxLoop genuinely does not know where in the
    /// order an administrator wants it ([ADR-0053]).
    ///
    /// [ADR-0053]: ../../../docs/adr/0053-the-loop-order-is-complete-and-a-new-loop-lands-at-the-end.md
    async fn create_loop(&mut self, name: &str) -> Result<LoopId, AdministrationRefused>;

    /// Read a loop by the id that identifies it.
    async fn a_loop(&mut self, id: &LoopId) -> Result<Option<Loop>, StoreError>;

    /// Every loop on the deployment, **in the administered base order**.
    ///
    /// There is no second read that answers them alphabetically. The order is one of the
    /// things being administered, so a read that quietly sorted would be answering a
    /// different question ([ADR-0053]).
    ///
    /// [ADR-0053]: ../../../docs/adr/0053-the-loop-order-is-complete-and-a-new-loop-lands-at-the-end.md
    async fn loops(&mut self) -> Result<Vec<Loop>, StoreError>;

    /// Change the name a loop is known by, leaving everything that references it alone.
    async fn rename_loop(
        &mut self,
        id: &LoopId,
        name: &str,
    ) -> Result<Option<Change<Loop>>, AdministrationRefused>;

    /// Delete a loop, and everything that is only about it.
    async fn delete_loop(&mut self, id: &LoopId) -> Result<Option<Change<Loop>>, StoreError>;

    /// Set the deployment-wide base order, as a complete ordering of the loops that exist.
    ///
    /// It answers with the order as it now stands. An order naming anything other than
    /// exactly the loops that are there is refused rather than half-applied, which is also
    /// what tells a console that was arranging an order while somebody else created a loop
    /// to read again.
    async fn set_the_loop_order(
        &mut self,
        order: &[LoopId],
    ) -> Result<Vec<Loop>, AdministrationRefused>;
}

#[async_trait]
impl Loops for Transaction {
    async fn create_loop(&mut self, name: &str) -> Result<LoopId, AdministrationRefused> {
        let id = LoopId(secrets::unguessable());

        sqlx::query(
            "INSERT INTO loops (id, name, is_unreviewed, position, created_at) \
             VALUES (?, ?, 1, (SELECT COALESCE(MAX(position), -1) + 1 FROM loops), ?)",
        )
        .bind(&id.0)
        .bind(name)
        .bind(now())
        .execute(self.connection())
        .await
        .map_err(|error| taken_or_unavailable(error, "loop name", name))?;

        Ok(id)
    }

    async fn a_loop(&mut self, id: &LoopId) -> Result<Option<Loop>, StoreError> {
        let found = sqlx::query("SELECT id, name, is_unreviewed FROM loops WHERE id = ?")
            .bind(&id.0)
            .fetch_optional(self.connection())
            .await
            .map_err(unavailable)?;

        Ok(found.as_ref().map(a_loop))
    }

    async fn loops(&mut self) -> Result<Vec<Loop>, StoreError> {
        let rows = sqlx::query(
            // `created_at` and the id break a tie the base order cannot: two loops share a
            // position only if a store was edited by hand, and an order that changes between
            // two reads of an unchanged store is worse than one nobody administered.
            "SELECT id, name, is_unreviewed FROM loops ORDER BY position, created_at, id",
        )
        .fetch_all(self.connection())
        .await
        .map_err(unavailable)?;

        Ok(rows.iter().map(a_loop).collect())
    }

    async fn rename_loop(
        &mut self,
        id: &LoopId,
        name: &str,
    ) -> Result<Option<Change<Loop>>, AdministrationRefused> {
        let Some(before) = self.a_loop(id).await? else {
            return Ok(None);
        };

        sqlx::query("UPDATE loops SET name = ? WHERE id = ?")
            .bind(name)
            .bind(&id.0)
            .execute(self.connection())
            .await
            .map_err(|error| taken_or_unavailable(error, "loop name", name))?;

        Ok(Some(Change {
            before,
            after: self.a_loop(id).await?,
        }))
    }

    async fn delete_loop(&mut self, id: &LoopId) -> Result<Option<Change<Loop>>, StoreError> {
        let Some(before) = self.a_loop(id).await? else {
            return Ok(None);
        };

        sqlx::query("DELETE FROM loops WHERE id = ?")
            .bind(&id.0)
            .execute(self.connection())
            .await
            .map_err(unavailable)?;

        Ok(Some(Change {
            before,
            // Nothing after it, which is the whole of what a deletion says. The loops around
            // it keep their positions: the order is what the positions say about each other,
            // never what any one of them is.
            after: None,
        }))
    }

    async fn set_the_loop_order(
        &mut self,
        order: &[LoopId],
    ) -> Result<Vec<Loop>, AdministrationRefused> {
        let present = self.loops().await?;

        if !names_every_loop_once(order, &present) {
            return Err(AdministrationRefused::IncompleteOrder);
        }

        for (position, id) in order.iter().enumerate() {
            sqlx::query("UPDATE loops SET position = ? WHERE id = ?")
                .bind(i64::try_from(position).unwrap_or(i64::MAX))
                .bind(&id.0)
                .execute(self.connection())
                .await
                .map_err(unavailable)?;
        }

        Ok(self.loops().await?)
    }
}

/// A loop, from the row every read of one selects.
fn a_loop(row: &sqlx::sqlite::SqliteRow) -> Loop {
    Loop {
        id: LoopId::known(row.get("id")),
        name: row.get("name"),
        is_unreviewed: row.get::<i64, _>("is_unreviewed") != 0,
    }
}

/// Whether this order is the complete ordering the base order has to be ([ADR-0053]).
///
/// [ADR-0053]: ../../../docs/adr/0053-the-loop-order-is-complete-and-a-new-loop-lands-at-the-end.md
fn names_every_loop_once(order: &[LoopId], present: &[Loop]) -> bool {
    let named: std::collections::HashSet<&str> = order.iter().map(LoopId::as_str).collect();

    named.len() == order.len()
        && named.len() == present.len()
        && present.iter().all(|held| named.contains(held.id.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::store::a_temporary_store;

    async fn loops_named(transaction: &mut Transaction, names: [&str; 3]) -> Vec<LoopId> {
        let mut made = Vec::new();
        for name in names {
            made.push(
                transaction
                    .create_loop(name)
                    .await
                    .expect("the loop to be created"),
            );
        }

        made
    }

    async fn order_of(transaction: &mut Transaction) -> Vec<String> {
        transaction
            .loops()
            .await
            .expect("the read to answer")
            .into_iter()
            .map(|held| held.name)
            .collect()
    }

    #[tokio::test]
    async fn install_leaves_a_deployment_with_no_loops_at_all() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");

        assert!(transaction.loops().await.expect("a read").is_empty());
    }

    #[tokio::test]
    async fn a_loop_created_after_install_carries_unreviewed() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");

        let id = transaction
            .create_loop("FLIGHT")
            .await
            .expect("the loop to be created");

        assert_eq!(
            transaction.a_loop(&id).await.expect("the read to answer"),
            Some(Loop {
                id,
                name: "FLIGHT".to_owned(),
                is_unreviewed: true,
            })
        );
    }

    #[tokio::test]
    async fn refuses_a_loop_name_already_taken_whatever_its_case() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        transaction.create_loop("FLIGHT").await.expect("the loop");

        let refusal = transaction.create_loop("flight").await;

        assert!(
            matches!(refusal, Err(AdministrationRefused::NameTaken { .. })),
            "expected the name to be refused, got {refusal:?}"
        );
    }

    #[tokio::test]
    async fn the_base_order_is_administered_rather_than_alphabetical_or_creation_order() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let made = loops_named(&mut transaction, ["GNC", "FLIGHT", "THERMAL"]).await;

        let administered = [made[2].clone(), made[0].clone(), made[1].clone()];
        transaction
            .set_the_loop_order(&administered)
            .await
            .expect("the order to be set");

        let order = order_of(&mut transaction).await;
        assert_eq!(order, ["THERMAL", "GNC", "FLIGHT"]);
        assert_ne!(
            order,
            ["FLIGHT", "GNC", "THERMAL"],
            "the order is alphabetical"
        );
        assert_ne!(
            order,
            ["GNC", "FLIGHT", "THERMAL"],
            "the order is creation order"
        );
    }

    #[tokio::test]
    async fn a_new_loop_lands_at_the_end_of_the_administered_order() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let made = loops_named(&mut transaction, ["GNC", "FLIGHT", "THERMAL"]).await;
        transaction
            .set_the_loop_order(&[made[2].clone(), made[0].clone(), made[1].clone()])
            .await
            .expect("the order to be set");

        transaction
            .create_loop("AIR")
            .await
            .expect("the loop to be created");

        assert_eq!(
            order_of(&mut transaction).await,
            ["THERMAL", "GNC", "FLIGHT", "AIR"],
            "a new loop was placed somewhere VoxLoop was never told to put it"
        );
    }

    #[tokio::test]
    async fn refuses_an_order_that_does_not_name_every_loop_exactly_once() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let made = loops_named(&mut transaction, ["GNC", "FLIGHT", "THERMAL"]).await;

        for attempt in [
            vec![made[0].clone(), made[1].clone()],
            vec![made[0].clone(), made[0].clone(), made[1].clone()],
            vec![
                made[0].clone(),
                made[1].clone(),
                made[2].clone(),
                LoopId::presented("no-such-loop".to_owned()),
            ],
        ] {
            let refusal = transaction.set_the_loop_order(&attempt).await;

            assert!(
                matches!(refusal, Err(AdministrationRefused::IncompleteOrder)),
                "expected an incomplete order to be refused, got {refusal:?}"
            );
        }

        assert_eq!(
            order_of(&mut transaction).await,
            ["GNC", "FLIGHT", "THERMAL"],
            "a refused order was applied anyway"
        );
    }

    #[tokio::test]
    async fn renaming_a_loop_leaves_the_id_and_its_place_in_the_order_alone() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let made = loops_named(&mut transaction, ["GNC", "FLIGHT", "THERMAL"]).await;

        let change = transaction
            .rename_loop(&made[1], "FLIGHT DIRECTOR")
            .await
            .expect("the rename to land")
            .expect("a change");

        assert_eq!(change.before.name, "FLIGHT");
        assert_eq!(
            change.after.expect("the loop after").name,
            "FLIGHT DIRECTOR"
        );
        assert_eq!(
            order_of(&mut transaction).await,
            ["GNC", "FLIGHT DIRECTOR", "THERMAL"]
        );
    }

    #[tokio::test]
    async fn deleting_a_loop_leaves_the_order_of_the_others_as_it_was() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let made = loops_named(&mut transaction, ["GNC", "FLIGHT", "THERMAL"]).await;

        let change = transaction
            .delete_loop(&made[0])
            .await
            .expect("the deletion to land")
            .expect("a change");

        assert_eq!(change.before.name, "GNC");
        assert!(change.after.is_none(), "a deletion left something behind");
        assert_eq!(order_of(&mut transaction).await, ["FLIGHT", "THERMAL"]);
    }

    #[tokio::test]
    async fn a_write_against_an_id_nobody_holds_is_no_change_rather_than_a_refusal() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let nobody = LoopId::presented("no-such-loop".to_owned());

        assert!(
            transaction
                .rename_loop(&nobody, "FLIGHT")
                .await
                .expect("the write to answer")
                .is_none()
        );
        assert!(
            transaction
                .delete_loop(&nobody)
                .await
                .expect("the write to answer")
                .is_none()
        );
    }
}
