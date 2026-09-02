//! Personalisation: everything a user has set about their own console that carries no
//! authority.
//!
//! One item lives here so far — the **subscription set**, scoped to (user, role) — and the
//! rules it is held under are the ones every later item inherits ([ADR-0050], [ADR-0051]).
//!
//! - **It is the memory of a live act, never the act.** A subscription is live state and
//!   ends with the session; what this module holds is the set a (user, role) pair last had,
//!   so that assuming rebuilds a console. The two are different things with different
//!   lifetimes, which is why nothing here is called *subscribe*.
//! - **It is written through as the live act is applied, best effort.** The write must never
//!   be able to fail a live act: if the live change lands and this does not, the console is
//!   correct and the preference is lost, which is the right way round. Nothing here decides
//!   that — the caller does — but it is why the operations are small enough to be attempted
//!   and dropped.
//! - **It grants nothing.** The grid overrules it silently and always ([ADR-0051]), so a
//!   remembered subscription to a loop the role has since lost `monitor` on is kept and
//!   inert rather than deleted: a temporary revocation must not destroy somebody's console
//!   arrangement, and a loop that leaves reach and comes back comes back where it was.
//! - **It is not audited.** A user changing their own personalisation is not a configuration
//!   change (v1 §10), and there is no endpoint to refuse either — the write rides the live
//!   act, so `docs/spec/api-surface.md` stays enumerable.
//!
//! [ADR-0050]: ../../../docs/adr/0050-personalisation-persists-what-is-safe-to-be-stale.md
//! [ADR-0051]: ../../../docs/adr/0051-personalisation-is-scoped-to-the-smallest-thing-it-is-about.md

use async_trait::async_trait;

use super::loops::LoopId;
use super::roles::RoleId;
use super::store::{StoreError, Transaction, now, unavailable};
use super::users::UserId;

/// Personalisation, as domain operations rather than queries ([ADR-0038]).
///
/// [ADR-0038]: ../../../docs/adr/0038-sqlite-behind-domain-shaped-repositories.md
#[async_trait]
pub(crate) trait Personalisation {
    /// The subscription set this pair last had, in the administered base loop order.
    ///
    /// It is the one read there is, and *assume* is the one caller: the set is handed to the
    /// state authority as a value, which is the only way the durable side and the live side
    /// ever meet ([ADR-0039]).
    ///
    /// **It is not narrowed to reach here.** A remembered subscription outside the role's
    /// reach is kept and inert ([ADR-0051]), and the narrowing happens where the document is
    /// projected — so a loop that leaves reach and returns comes back where it was. Narrowing
    /// on the way out would be the same as deleting it, one assume later.
    ///
    /// An empty answer is a pair with no subscriptions. It does not say whether they ever
    /// had any: the role default that would need that distinction is #27's.
    ///
    /// [ADR-0039]: ../../../docs/adr/0039-live-state-is-in-process-behind-one-state-authority.md
    /// [ADR-0051]: ../../../docs/adr/0051-personalisation-is-scoped-to-the-smallest-thing-it-is-about.md
    async fn the_subscriptions_of(
        &mut self,
        user: &UserId,
        role: &RoleId,
    ) -> Result<Vec<LoopId>, StoreError>;

    /// Remember that this pair has this loop up.
    ///
    /// Idempotent, and deliberately: the live act it rides is applied to a set, so a second
    /// subscribe to a loop already held is the same state rather than a second one. The
    /// original time is left alone, because when somebody first put a loop up is the more
    /// useful of the two answers and neither is read today.
    ///
    /// A triple naming a record that is not there writes nothing, exactly as every other
    /// write against an id nobody holds does.
    async fn remember_a_subscription(
        &mut self,
        user: &UserId,
        role: &RoleId,
        held_on: &LoopId,
    ) -> Result<(), StoreError>;

    /// Forget that this pair had this loop up.
    ///
    /// Idempotent for the same reason, and it answers nothing: there is no *was it there*
    /// for a caller to act on, because the live set is the thing that decides and this only
    /// ever follows it.
    async fn forget_a_subscription(
        &mut self,
        user: &UserId,
        role: &RoleId,
        held_on: &LoopId,
    ) -> Result<(), StoreError>;
}

#[async_trait]
impl Personalisation for Transaction {
    async fn the_subscriptions_of(
        &mut self,
        user: &UserId,
        role: &RoleId,
    ) -> Result<Vec<LoopId>, StoreError> {
        // Joined to `loops` for the order rather than read on its own: the base order is
        // administered (ADR-0053), and a set read in whatever order the index happened to
        // hand back would be a second order nobody set.
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT remembered_subscriptions.loop_id FROM remembered_subscriptions \
             JOIN loops ON loops.id = remembered_subscriptions.loop_id \
             WHERE remembered_subscriptions.user_id = ? AND remembered_subscriptions.role_id = ? \
             ORDER BY loops.position",
        )
        .bind(user.as_str())
        .bind(role.as_str())
        .fetch_all(self.connection())
        .await
        .map_err(unavailable)?;

        Ok(rows.into_iter().map(|(id,)| LoopId::known(id)).collect())
    }

    async fn remember_a_subscription(
        &mut self,
        user: &UserId,
        role: &RoleId,
        held_on: &LoopId,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO remembered_subscriptions (user_id, role_id, loop_id, subscribed_at) \
             SELECT ?, ?, ?, ? \
             WHERE EXISTS (SELECT 1 FROM users WHERE id = ?) \
               AND EXISTS (SELECT 1 FROM roles WHERE id = ?) \
               AND EXISTS (SELECT 1 FROM loops WHERE id = ?) \
             ON CONFLICT (user_id, role_id, loop_id) DO NOTHING",
        )
        .bind(user.as_str())
        .bind(role.as_str())
        .bind(held_on.as_str())
        .bind(now())
        .bind(user.as_str())
        .bind(role.as_str())
        .bind(held_on.as_str())
        .execute(self.connection())
        .await
        .map_err(unavailable)?;

        Ok(())
    }

    async fn forget_a_subscription(
        &mut self,
        user: &UserId,
        role: &RoleId,
        held_on: &LoopId,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "DELETE FROM remembered_subscriptions \
             WHERE user_id = ? AND role_id = ? AND loop_id = ?",
        )
        .bind(user.as_str())
        .bind(role.as_str())
        .bind(held_on.as_str())
        .execute(self.connection())
        .await
        .map_err(unavailable)?;

        Ok(())
    }
}

/// Make every subscription write fail, for as long as this store lives.
///
/// **A test-only hole in the seam, and the only way to prove the rule that matters here**:
/// personalisation is best effort and must never be able to fail a live act ([ADR-0050]).
/// That is a claim about what happens when this write is the thing that breaks, and there is
/// no in-memory fake to break ([ADR-0064]) — so the real store is made to refuse this one
/// table, the way the audit log's own triggers refuse an amendment.
///
/// It is here rather than in the test that uses it because `Transaction::connection` is this
/// module's alone, which is the whole of what makes the repository seam a seam.
#[cfg(test)]
pub(crate) async fn refuse_every_subscription_write(transaction: &mut Transaction) {
    // Written out rather than built, because a query this module assembles from pieces is
    // the one thing `sqlx` refuses outright — and it is right to: a seam that can compose a
    // statement is a seam that can compose the wrong one.
    for statement in [
        "CREATE TRIGGER remembered_subscriptions_refuse_inserts \
         BEFORE INSERT ON remembered_subscriptions \
         BEGIN SELECT RAISE(ABORT, 'the store is unwell'); END",
        "CREATE TRIGGER remembered_subscriptions_refuse_deletes \
         BEFORE DELETE ON remembered_subscriptions \
         BEGIN SELECT RAISE(ABORT, 'the store is unwell'); END",
    ] {
        sqlx::query(statement)
            .execute(transaction.connection())
            .await
            .expect("the trigger to be created");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::loops::Loops;
    use crate::configuration::roles::{NewRole, Roles};
    use crate::configuration::store::a_temporary_store;
    use crate::configuration::users::{NewUser, Users};

    /// A user, a role and three loops in an order nobody would arrive at by sorting.
    async fn a_deployment(transaction: &mut Transaction) -> (UserId, RoleId, Vec<LoopId>) {
        let user = transaction
            .create_user(NewUser {
                username: "flight".to_owned(),
                password_hash: None,
                is_system_administrator: false,
            })
            .await
            .expect("the user to be created");
        let role = transaction
            .create_role(NewRole {
                name: "Flight Director".to_owned(),
                max_occupants: Some(1),
            })
            .await
            .expect("the role to be created");

        let mut loops = Vec::new();
        for name in ["THERMAL", "FLIGHT", "GNC"] {
            loops.push(
                transaction
                    .create_loop(name)
                    .await
                    .expect("the loop to be created"),
            );
        }

        (user, role, loops)
    }

    #[tokio::test]
    async fn a_pair_nobody_has_personalised_has_no_subscriptions() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let (user, role, _loops) = a_deployment(&mut transaction).await;

        assert!(
            transaction
                .the_subscriptions_of(&user, &role)
                .await
                .expect("the set to be readable")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn remembers_a_subscription_and_forgets_it_again() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let (user, role, loops) = a_deployment(&mut transaction).await;

        transaction
            .remember_a_subscription(&user, &role, &loops[1])
            .await
            .expect("the write to land");
        assert_eq!(
            transaction
                .the_subscriptions_of(&user, &role)
                .await
                .expect("the set to be readable"),
            [loops[1].clone()]
        );

        transaction
            .forget_a_subscription(&user, &role, &loops[1])
            .await
            .expect("the write to land");
        assert!(
            transaction
                .the_subscriptions_of(&user, &role)
                .await
                .expect("the set to be readable")
                .is_empty()
        );
    }

    /// The set is a set: a second subscribe to a loop already held is the same state, and the
    /// live act it rides is applied to a set too.
    #[tokio::test]
    async fn remembering_the_same_loop_twice_is_one_subscription() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let (user, role, loops) = a_deployment(&mut transaction).await;

        for _ in 0..2 {
            transaction
                .remember_a_subscription(&user, &role, &loops[0])
                .await
                .expect("the write to land");
        }

        assert_eq!(
            transaction
                .the_subscriptions_of(&user, &role)
                .await
                .expect("the set to be readable"),
            [loops[0].clone()]
        );
    }

    /// Forgetting what was never remembered is not a failure. The live set decides and this
    /// follows it, so there is no *was it there* for a caller to act on.
    #[tokio::test]
    async fn forgetting_a_subscription_nobody_held_changes_nothing() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let (user, role, loops) = a_deployment(&mut transaction).await;

        transaction
            .forget_a_subscription(&user, &role, &loops[2])
            .await
            .expect("the write to land");

        assert!(
            transaction
                .the_subscriptions_of(&user, &role)
                .await
                .expect("the set to be readable")
                .is_empty()
        );
    }

    /// The base loop order is administered rather than derived (ADR-0053), so the set comes
    /// back in it. Reading it in whatever order the key happened to hand back would be a
    /// second order nobody set.
    #[tokio::test]
    async fn answers_the_set_in_the_administered_base_order() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let (user, role, loops) = a_deployment(&mut transaction).await;
        transaction
            .set_the_loop_order(&[loops[2].clone(), loops[0].clone(), loops[1].clone()])
            .await
            .expect("the order to be set");

        for held_on in loops.iter().rev() {
            transaction
                .remember_a_subscription(&user, &role, held_on)
                .await
                .expect("the write to land");
        }

        assert_eq!(
            transaction
                .the_subscriptions_of(&user, &role)
                .await
                .expect("the set to be readable"),
            [loops[2].clone(), loops[0].clone(), loops[1].clone()]
        );
    }

    /// Personalisation is scoped to (user, role) and never to the person (ADR-0051): the
    /// loops somebody has up are a property of the seat they are in.
    #[tokio::test]
    async fn one_user_two_roles_are_two_subscription_sets() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let (user, role, loops) = a_deployment(&mut transaction).await;
        let other_role = transaction
            .create_role(NewRole {
                name: "CAPCOM".to_owned(),
                max_occupants: None,
            })
            .await
            .expect("the role to be created");

        transaction
            .remember_a_subscription(&user, &role, &loops[0])
            .await
            .expect("the write to land");

        assert!(
            transaction
                .the_subscriptions_of(&user, &other_role)
                .await
                .expect("the set to be readable")
                .is_empty()
        );
    }

    /// **Deleting a loop takes the personalisation about it with it** (ADR-0050). There is
    /// nothing to preserve once the thing being personalised is gone, and a row referencing
    /// it would be a subscription to a loop that does not exist.
    #[tokio::test]
    async fn deleting_a_loop_takes_the_subscriptions_to_it() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let (user, role, loops) = a_deployment(&mut transaction).await;
        for held_on in &loops {
            transaction
                .remember_a_subscription(&user, &role, held_on)
                .await
                .expect("the write to land");
        }

        transaction
            .delete_loop(&loops[0])
            .await
            .expect("the loop to be deleted");

        assert_eq!(
            transaction
                .the_subscriptions_of(&user, &role)
                .await
                .expect("the set to be readable"),
            [loops[1].clone(), loops[2].clone()]
        );
    }

    /// A triple naming a record nobody holds writes nothing, exactly as every other write
    /// against an id nobody holds does — rather than a foreign key refusing it, which is not
    /// an answer this seam is allowed to hand back.
    #[tokio::test]
    async fn a_subscription_to_a_loop_nobody_holds_is_no_change() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let (user, role, _loops) = a_deployment(&mut transaction).await;

        transaction
            .remember_a_subscription(&user, &role, &LoopId::presented("nothing".to_owned()))
            .await
            .expect("the write to land");

        assert!(
            transaction
                .the_subscriptions_of(&user, &role)
                .await
                .expect("the set to be readable")
                .is_empty()
        );
    }
}
