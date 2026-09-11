//! Personalisation: everything a user has set about their own console that carries no
//! authority.
//!
//! Two items live here so far — the **subscription set**, scoped to (user, role), and the
//! **per-loop volume**, scoped to (user, role, loop) — and the rules they are held under are
//! the ones every later item inherits ([ADR-0050], [ADR-0051]).
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

/// How loud one loop plays in one operator's ears: a percentage of full gain.
///
/// **It is an attenuation control** (v1 §5), so it runs from silence up to unity and no
/// further. An operator turns a loop down because it is chatter they do not need moment to
/// moment; there is no turning one up past what it is, and a value that claimed to would be
/// one the console could not play.
///
/// **Every loop starts at unity** (v1 §10). That is the default and it is not a role's to
/// move: an administrator setting a level for everyone is guessing at headsets and hearing
/// ([ADR-0052]).
///
/// [ADR-0052]: ../../../docs/adr/0052-a-role-default-is-a-starting-point-never-a-floor.md
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Volume(u8);

impl Volume {
    /// Full gain, which is where every loop starts.
    pub(crate) const UNITY: Self = Self(100);

    /// A volume somebody asked for, where it is one. Anything past unity is not.
    pub(crate) fn presented(percent: u8) -> Option<Self> {
        (percent <= Self::UNITY.0).then_some(Self(percent))
    }

    /// The percentage, which is how the console shows it and how the store holds it.
    pub(crate) fn percent(self) -> u8 {
        self.0
    }
}

impl Default for Volume {
    fn default() -> Self {
        Self::UNITY
    }
}

/// Personalisation, as domain operations rather than queries ([ADR-0038]).
///
/// [ADR-0038]: ../../../docs/adr/0038-sqlite-behind-domain-shaped-repositories.md
#[async_trait]
pub(crate) trait Personalisation {
    /// The subscription set this pair last had, in the administered base loop order.
    ///
    /// *Assume* is the one caller: the set is handed to the state authority as a value,
    /// which is the only way the durable side and the live side ever meet ([ADR-0039]).
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
    /// useful of the two answers.
    ///
    /// Nothing reads that time. It is there because every record this store holds carries
    /// when it was made, and the store is a file somebody reads by hand when a deployment is
    /// behaving oddly — a row that cannot say when it appeared is one nobody can place.
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

    /// The volumes this pair has set, one per loop somebody has touched.
    ///
    /// **A loop missing from the answer is at unity**, which is where every loop starts
    /// (v1 §10). Like the subscription set it is read at *assume* and handed to the state
    /// authority as a value, and like it, it is **not narrowed to reach**: a volume on a loop
    /// the role has since lost is kept and inert, so the loop comes back at the level it left
    /// at ([ADR-0051]).
    ///
    /// [ADR-0051]: ../../../docs/adr/0051-personalisation-is-scoped-to-the-smallest-thing-it-is-about.md
    async fn the_volumes_of(
        &mut self,
        user: &UserId,
        role: &RoleId,
    ) -> Result<Vec<(LoopId, Volume)>, StoreError>;

    /// Remember the volume this pair has set on this loop, replacing whatever it had.
    ///
    /// **Volume persists and mute does not** ([ADR-0050]), and the difference is what a
    /// stale value costs. A loop left turned down is one the operator sees on the card the
    /// moment they assume; a loop left muted drops every loop they staff to `away` for the
    /// whole operations centre before they have looked at anything. So there is no mute here
    /// to remember, and nothing that could write one.
    ///
    /// A triple naming a record that is not there writes nothing.
    ///
    /// [ADR-0050]: ../../../docs/adr/0050-personalisation-persists-what-is-safe-to-be-stale.md
    async fn remember_a_volume(
        &mut self,
        user: &UserId,
        role: &RoleId,
        held_on: &LoopId,
        volume: Volume,
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

    async fn the_volumes_of(
        &mut self,
        user: &UserId,
        role: &RoleId,
    ) -> Result<Vec<(LoopId, Volume)>, StoreError> {
        // In the administered base order, for the reason the subscription set is (ADR-0053):
        // an answer in whatever order the key came back in would be a second order nobody set.
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT remembered_volumes.loop_id, remembered_volumes.volume \
             FROM remembered_volumes \
             JOIN loops ON loops.id = remembered_volumes.loop_id \
             WHERE remembered_volumes.user_id = ? AND remembered_volumes.role_id = ? \
             ORDER BY loops.position",
        )
        .bind(user.as_str())
        .bind(role.as_str())
        .fetch_all(self.connection())
        .await
        .map_err(unavailable)?;

        // The column's own check holds every row to the range, so a row that is not a volume
        // is one written by something other than this binary. It is left out rather than
        // clamped: guessing at what somebody meant would be setting a level nobody chose.
        Ok(rows
            .into_iter()
            .filter_map(|(id, percent)| {
                let volume = u8::try_from(percent).ok().and_then(Volume::presented)?;
                Some((LoopId::known(id), volume))
            })
            .collect())
    }

    async fn remember_a_volume(
        &mut self,
        user: &UserId,
        role: &RoleId,
        held_on: &LoopId,
        volume: Volume,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO remembered_volumes (user_id, role_id, loop_id, volume, set_at) \
             SELECT ?, ?, ?, ?, ? \
             WHERE EXISTS (SELECT 1 FROM users WHERE id = ?) \
               AND EXISTS (SELECT 1 FROM roles WHERE id = ?) \
               AND EXISTS (SELECT 1 FROM loops WHERE id = ?) \
             ON CONFLICT (user_id, role_id, loop_id) \
             DO UPDATE SET volume = excluded.volume, set_at = excluded.set_at",
        )
        .bind(user.as_str())
        .bind(role.as_str())
        .bind(held_on.as_str())
        .bind(i64::from(volume.percent()))
        .bind(now())
        .bind(user.as_str())
        .bind(role.as_str())
        .bind(held_on.as_str())
        .execute(self.connection())
        .await
        .map_err(unavailable)?;

        Ok(())
    }
}

/// Make every personalisation write fail, for as long as this store lives.
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
pub(crate) async fn refuse_every_personalisation_write(transaction: &mut Transaction) {
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
        "CREATE TRIGGER remembered_volumes_refuse_inserts \
         BEFORE INSERT ON remembered_volumes \
         BEGIN SELECT RAISE(ABORT, 'the store is unwell'); END",
        "CREATE TRIGGER remembered_volumes_refuse_updates \
         BEFORE UPDATE ON remembered_volumes \
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

    // ---- Per-loop volume (#44) ------------------------------------------------------------

    /// **Every loop starts at unity** (v1 §10), and that is a pair with nothing remembered
    /// rather than a row per loop saying so.
    #[tokio::test]
    async fn a_pair_nobody_has_personalised_has_every_loop_at_unity() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let (user, role, _loops) = a_deployment(&mut transaction).await;

        assert!(
            transaction
                .the_volumes_of(&user, &role)
                .await
                .expect("the volumes to be readable")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn remembers_a_volume_and_answers_it_on_the_next_read() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let (user, role, loops) = a_deployment(&mut transaction).await;
        let turned_down = Volume::presented(40).expect("a volume");

        transaction
            .remember_a_volume(&user, &role, &loops[1], turned_down)
            .await
            .expect("the write to land");

        assert_eq!(
            transaction
                .the_volumes_of(&user, &role)
                .await
                .expect("the volumes to be readable"),
            [(loops[1].clone(), turned_down)]
        );
    }

    /// A loop has one volume, so setting it again is the new value rather than a second row.
    #[tokio::test]
    async fn setting_a_volume_again_replaces_it() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let (user, role, loops) = a_deployment(&mut transaction).await;

        for percent in [40, 75] {
            transaction
                .remember_a_volume(&user, &role, &loops[0], Volume::presented(percent).unwrap())
                .await
                .expect("the write to land");
        }

        assert_eq!(
            transaction
                .the_volumes_of(&user, &role)
                .await
                .expect("the volumes to be readable"),
            [(loops[0].clone(), Volume::presented(75).unwrap())]
        );
    }

    /// Volume is scoped per (user, role, loop), and the role is part of it (ADR-0051): the
    /// same person in another seat starts that seat's loops at unity.
    #[tokio::test]
    async fn one_user_two_roles_are_two_sets_of_volumes() {
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
            .remember_a_volume(&user, &role, &loops[0], Volume::presented(10).unwrap())
            .await
            .expect("the write to land");

        assert!(
            transaction
                .the_volumes_of(&user, &other_role)
                .await
                .expect("the volumes to be readable")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn deleting_a_loop_takes_its_volume_with_it() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let (user, role, loops) = a_deployment(&mut transaction).await;
        transaction
            .remember_a_volume(&user, &role, &loops[0], Volume::presented(10).unwrap())
            .await
            .expect("the write to land");

        transaction
            .delete_loop(&loops[0])
            .await
            .expect("the loop to be deleted");

        assert!(
            transaction
                .the_volumes_of(&user, &role)
                .await
                .expect("the volumes to be readable")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn a_volume_on_a_loop_nobody_holds_is_no_change() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let (user, role, _loops) = a_deployment(&mut transaction).await;

        transaction
            .remember_a_volume(
                &user,
                &role,
                &LoopId::presented("nothing".to_owned()),
                Volume::presented(10).unwrap(),
            )
            .await
            .expect("the write to land");

        assert!(
            transaction
                .the_volumes_of(&user, &role)
                .await
                .expect("the volumes to be readable")
                .is_empty()
        );
    }

    /// **Volume is an attenuation control** (v1 §5): it runs from silence up to unity and
    /// no further, so a value past unity is not a volume at all.
    #[test]
    fn a_volume_runs_from_silence_to_unity_and_no_further() {
        assert_eq!(Volume::presented(0).map(Volume::percent), Some(0));
        assert_eq!(Volume::presented(100), Some(Volume::UNITY));
        assert_eq!(Volume::presented(101), None);
        assert_eq!(Volume::default(), Volume::UNITY);
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
