//! The bootstrap code: the one credential nobody administered.
//!
//! There are no default credentials, ever ([ADR-0025]). On a start with no system
//! administrator in the store, VoxLoop mints a one-time code to its own log; whoever can
//! read the box's console redeems it once, from a browser, to create the first
//! administrator. **The root of trust is being on the box** — and the corollary, stated
//! rather than discovered, is that reading the server's log is equivalent to being an
//! administrator at that moment.
//!
//! The code lives for one run of the process and nowhere else, which is what makes every
//! start invalidate the code the start before it minted.
//!
//! [ADR-0025]: ../../../docs/adr/0025-credentials-are-administered-because-there-is-no-email.md

use std::sync::Mutex;

use crate::configuration::{Store, StoreError, Users};
use crate::secrets;
use crate::telemetry::module;

/// The code this run of the process minted, until somebody spends it.
pub(crate) struct Bootstrap {
    code: Mutex<Option<String>>,
}

/// What came of presenting a code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Redemption {
    /// It was the code, and it is now spent.
    Redeemed,
    /// It was not the code, or the code is already spent.
    Refused,
}

impl Bootstrap {
    /// Mint a code for this run, unless somebody already administers this deployment.
    ///
    /// `None` is the ordinary state of an established deployment, and it is what makes the
    /// redemption route stop existing rather than start refusing: a route the server never
    /// registered is the one exception to VoxLoop's rule that refusals say *you may not*
    /// rather than hiding an operation (v1 §3).
    pub(crate) async fn mint_unless_administered(
        store: &Store,
    ) -> Result<Option<Self>, StoreError> {
        let mut transaction = store.begin().await?;
        let administered = transaction.a_system_administrator_exists().await?;
        transaction.roll_back().await?;

        if administered {
            return Ok(None);
        }

        let code = secrets::unguessable();

        // Warned rather than informed: a deployment nobody administers yet is not a state to
        // run in, and a log level that hid the code would leave the box unopenable.
        tracing::warn!(
            target: module::IDENTITY,
            %code,
            "no system administrator: POST this code to /api/bootstrap with a username and \
             password to create one. It is good until this process stops."
        );

        Ok(Some(Self {
            code: Mutex::new(Some(code)),
        }))
    }

    /// The code this run minted, read the way only a test may.
    ///
    /// Everywhere else it is write-once: it goes to the log at mint and is compared, never
    /// handed back.
    #[cfg(test)]
    pub(crate) fn code(&self) -> Option<String> {
        self.code
            .lock()
            .unwrap_or_else(|held| held.into_inner())
            .clone()
    }

    /// Spend the code, if this is it.
    ///
    /// A wrong guess does not spend it. Otherwise anyone who could reach the box could put
    /// the deployment beyond bootstrapping with one request, which is a denial of service
    /// against the operator rather than a defence against the guesser — that is what rate
    /// limiting is for.
    pub(crate) fn redeem(&self, presented: &str) -> Redemption {
        let mut held = self.code.lock().unwrap_or_else(|held| held.into_inner());

        let Some(code) = held.as_deref() else {
            return Redemption::Refused;
        };

        if !secrets::are_the_same(presented, code) {
            return Redemption::Refused;
        }

        *held = None;
        Redemption::Redeemed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::{NewUser, a_temporary_store};

    fn code_of(bootstrap: &Bootstrap) -> String {
        bootstrap.code().expect("an unspent code")
    }

    #[tokio::test]
    async fn mints_a_code_while_nobody_administers_the_deployment() {
        let (_directory, store) = a_temporary_store().await;

        let bootstrap = Bootstrap::mint_unless_administered(&store)
            .await
            .expect("the store to answer");

        assert!(bootstrap.is_some());
    }

    #[tokio::test]
    async fn mints_nothing_once_somebody_does() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        transaction
            .create_user(NewUser {
                username: "root".to_owned(),
                password_hash: None,
                is_system_administrator: true,
            })
            .await
            .expect("an administrator");
        transaction
            .commit()
            .await
            .expect("the administrator to land");

        let bootstrap = Bootstrap::mint_unless_administered(&store)
            .await
            .expect("the store to answer");

        assert!(bootstrap.is_none());
    }

    #[tokio::test]
    async fn every_start_invalidates_the_code_the_start_before_it_minted() {
        let (_directory, store) = a_temporary_store().await;

        let first = Bootstrap::mint_unless_administered(&store)
            .await
            .expect("the store to answer")
            .expect("a code");
        let second = Bootstrap::mint_unless_administered(&store)
            .await
            .expect("the store to answer")
            .expect("a code");

        let minted_first = code_of(&first);
        assert_ne!(minted_first, code_of(&second));
        assert_eq!(
            second.redeem(&minted_first),
            Redemption::Refused,
            "the previous start's code still works"
        );
    }

    #[tokio::test]
    async fn the_code_is_good_once() {
        let (_directory, store) = a_temporary_store().await;
        let bootstrap = Bootstrap::mint_unless_administered(&store)
            .await
            .expect("the store to answer")
            .expect("a code");
        let code = code_of(&bootstrap);

        assert_eq!(bootstrap.redeem(&code), Redemption::Redeemed);
        assert_eq!(bootstrap.redeem(&code), Redemption::Refused);
    }

    #[tokio::test]
    async fn a_wrong_guess_does_not_spend_the_code() {
        let (_directory, store) = a_temporary_store().await;
        let bootstrap = Bootstrap::mint_unless_administered(&store)
            .await
            .expect("the store to answer")
            .expect("a code");
        let code = code_of(&bootstrap);

        assert_eq!(bootstrap.redeem("not the code"), Redemption::Refused);
        assert_eq!(bootstrap.redeem(&code), Redemption::Redeemed);
    }
}
