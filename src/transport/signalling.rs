//! The signalling channel, and the lobby it carries.
//!
//! **One socket per tab, opened at sign-in**, starting at `SignedIn` (ADR-0054). It is the
//! one channel live state travels on — the media transport carries audio and nothing else
//! ([ADR-0019]) — and it is a **second authorised surface**, checked **per message and not at
//! the upgrade**.
//!
//! Upgrade-time authorisation is the tempting shortcut and it breaks the moment an
//! administrator edits a grid cell mid-shift: the socket is already open, and a revoked
//! `emit` would keep arming until the operator happened to reconnect. So every message
//! carries a requirement and every requirement is evaluated against the store as it stands
//! at that moment, which is the same rule HTTP routes hold and the same evaluator behind it.
//!
//! **The upgrade refuses a service token.** It is registered `SignedIn`, which no token can
//! satisfy, and a request presenting a cookie and a token together is refused rather than
//! resolved by precedence — a service principal has no session, no client and no media path
//! ([ADR-0029]).
//!
//! What the socket carries today is the **lobby**: read-only, no audio, no authority, no
//! talking indicators. It answers one question — *should I assume a role, and which?* — and
//! deliberately nothing more ([ADR-0023]). Assuming the role, and the presence document that
//! follows, are #37's.
//!
//! [ADR-0019]: ../../../docs/adr/0019-presence-is-one-versioned-document-scoped-to-reach.md
//! [ADR-0023]: ../../../docs/adr/0023-sign-in-is-to-the-application-and-a-role-is-assumed.md
//! [ADR-0029]: ../../../docs/adr/0029-an-announcement-is-an-ordinary-transmission.md

use std::time::Duration;

use axum::Extension;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use serde::{Deserialize, Serialize};

use super::{Api, answers, unmet};
use crate::authorisation::{self, Caller, Outcome, Presented, Requirement};
use crate::configuration::{
    Eligibilities, Role, SignInToken, SignIns, StoreError, Transaction, UserId, Users,
};
use crate::telemetry::module;

/// How often the lobby is worked out again and pushed if it has moved.
///
/// Slower than the presence document's tick on purpose: everything in the lobby changes at
/// human speed — somebody takes a seat, an administrator grants an eligibility — and there
/// is no audio and no authority riding on it. The document is only sent when it differs from
/// the one before, so a quiet deployment sends nothing at all.
const TICK: Duration = Duration::from_secs(1);

/// Open the signalling channel. `SignedIn`. Not audited: opening it is not a decision, and
/// the sign-in it stands on already is.
pub(super) async fn open(
    State(api): State<Api>,
    Extension(caller): Extension<Caller>,
    upgrade: WebSocketUpgrade,
) -> Response {
    let Caller::User { id, sign_in } = caller else {
        // Unreachable: the requirement resolved a user before this handler ran.
        return answers::refusal("That operation is for a signed-in user.");
    };

    upgrade.on_upgrade(move |socket| async move {
        Conversation::opened(api, id, sign_in).talk(socket).await;
    })
}

/// One socket, and everything it needs to answer for itself.
struct Conversation {
    api: Api,
    /// Whose tab this is. Resolved at the upgrade and never trusted afterwards: every
    /// message re-reads the sign-in, so the flag, the lock and the eligibility a message is
    /// answered under are the ones the store holds at that moment.
    user: UserId,
    sign_in: SignInToken,
    /// Monotonic per socket, and it moves only when the document does.
    ///
    /// A version that ticked whether or not anything changed would make *is this the same
    /// state* unanswerable, which is the one question versioning is for ([ADR-0019]).
    ///
    /// [ADR-0019]: ../../../docs/adr/0019-presence-is-one-versioned-document-scoped-to-reach.md
    version: u64,
    /// The last document this socket sent, to tell a change from a redundant send.
    sent: Option<Lobby>,
}

/// What the client says to the server.
///
/// **Every message names its requirement** ([`Incoming::requirement`]), and the match is
/// exhaustive, so a message nobody ruled on does not compile — the same mechanism route
/// registration has, one level down.
#[derive(Deserialize)]
#[serde(tag = "message", rename_all = "kebab-case")]
enum Incoming {
    /// The client saying it has arrived and is ready to render.
    ///
    /// The server answers with the lobby document rather than pushing one at a socket that
    /// may not be listening yet. It performs the two rows `docs/spec/api-surface.md` gives
    /// this tier — opening the channel, and the lobby document it carries — rather than
    /// being a third operation of its own.
    ///
    /// #50 extends it to present a session id, which is what moves the socket from
    /// `SignedIn` to `Session` (ADR-0054) and what makes it the *resume a session* row too.
    Hello,
}

impl Incoming {
    /// What this message demands of whoever sent it.
    ///
    /// It is a function on the message rather than a field beside it, so adding a message
    /// without ruling on it is a build failure ([ADR-0054]). Nothing defaults to open here
    /// either.
    ///
    /// [ADR-0054]: ../../../docs/adr/0054-every-operation-declares-its-authorisation.md
    fn requirement(&self) -> Requirement {
        match self {
            Self::Hello => Requirement::SignedIn,
        }
    }

    /// The name for a refusal to say back, so an operator is told which message it was about.
    fn named(&self) -> &'static str {
        match self {
            Self::Hello => "hello",
        }
    }
}

/// What the server says to the client.
#[derive(Serialize, Debug, PartialEq)]
#[serde(tag = "message", rename_all = "kebab-case")]
enum Outgoing {
    /// The lobby, whole. It is rendered atomically and never merged into what is on screen
    /// ([ADR-0019]).
    ///
    /// [ADR-0019]: ../../../docs/adr/0019-presence-is-one-versioned-document-scoped-to-reach.md
    Lobby {
        version: u64,
        #[serde(flatten)]
        lobby: Lobby,
    },
    /// The caller may not, and this is what they did not meet. It says which message it is
    /// about, because a socket answers several and a bare reason would be unattributable.
    Refused { was: String, reason: String },
    /// The socket is going away, and why. A client that is merely disconnected cannot tell
    /// *ended* from *lost* on its own, and the two want different things of the operator.
    Closing { reason: String },
}

/// The lobby, as the console renders it.
///
/// The roles this user may assume and who is in each seat, and **nothing else**. No loops, no
/// reach, no talking indicators: the user holds no authority while standing here, which is
/// the whole reason this is allowed to read across roles at all ([ADR-0023]).
///
/// The staffing state of the loops those roles staff belongs here too, ledger-style with its
/// reason in full, and arrives with #48 — the lobby is read once and deliberately, by
/// somebody about to be in a position to fix what it says.
///
/// [ADR-0023]: ../../../docs/adr/0023-sign-in-is-to-the-application-and-a-role-is-assumed.md
#[derive(Serialize, Debug, Clone, PartialEq)]
struct Lobby {
    roles: Vec<Seat>,
}

/// One role a user may assume, and who is in it.
#[derive(Serialize, Debug, Clone, PartialEq)]
struct Seat {
    id: String,
    name: String,
    /// How many may occupy it at once, or `null` for a role with no limit ([ADR-0068]).
    ///
    /// It is here because the lobby cannot ask its question without it: an occupied
    /// single-occupant role is a seat that cannot be shared, and one somebody may only
    /// *request* from its incumbent. That request is issued **from the lobby against a
    /// single-occupant role** (`docs/spec/api-surface.md`), so the lobby has to be able to
    /// tell one kind of occupied seat from the other.
    ///
    /// [ADR-0068]: ../../../docs/adr/0068-a-role-with-no-limit-is-the-limit-left-unset.md
    max_occupants: Option<u32>,
    /// Who occupies it, by name. Occupancy means a role somebody has assumed and not
    /// relinquished, never somebody merely signed in ([ADR-0005]).
    ///
    /// [ADR-0005]: ../../../docs/adr/0005-occupancy-means-listening-not-signed-in.md
    occupants: Vec<String>,
}

impl Conversation {
    fn opened(api: Api, user: UserId, sign_in: SignInToken) -> Self {
        Self {
            api,
            user,
            sign_in,
            version: 0,
            sent: None,
        }
    }

    /// Carry one socket until it goes away, or until the sign-in behind it does.
    async fn talk(mut self, mut socket: WebSocket) {
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            let said = tokio::select! {
                received = socket.recv() => match received {
                    // The tab was closed, or the channel was lost. Neither ends anything a
                    // signed-in user holds: a sign-in survives, and so does a session for
                    // its reconnection window (#50).
                    None | Some(Err(_)) => break,
                    Some(Ok(Message::Text(said))) => self.received(&said).await,
                    // Ping, pong, binary and close: nothing VoxLoop says anything in.
                    Some(Ok(_)) => Ok(None),
                },
                _ = tick.tick() => self.pushed().await,
            };

            let said = match said {
                Ok(None) => continue,
                Ok(Some(said)) => said,
                Err(error) => {
                    tracing::error!(target: module::TRANSPORT, %error, "the socket could not be answered");
                    Outgoing::Closing {
                        reason: "VoxLoop could not answer that just now.".to_owned(),
                    }
                }
            };

            let closing = matches!(said, Outgoing::Closing { .. });
            if self.say(&mut socket, &said).await.is_err() || closing {
                break;
            }
        }
    }

    /// Answer one message from the client.
    ///
    /// The requirement is evaluated **now**, against the store, for this message. A message
    /// needing a session arriving on a lobby-tier socket is refused by this same check, and
    /// so is one from a sign-in that ended a second ago.
    async fn received(&mut self, said: &str) -> Result<Option<Outgoing>, StoreError> {
        let Ok(message) = serde_json::from_str::<Incoming>(said) else {
            // Nothing defaults to open, and that includes a message nobody has ruled on:
            // the socket does not guess what an unknown name meant.
            return Ok(Some(Outgoing::Refused {
                was: "that message".to_owned(),
                reason: "VoxLoop has no message by that name.".to_owned(),
            }));
        };

        if !self.permitted(&message.requirement()).await? {
            return Ok(Some(Outgoing::Refused {
                was: message.named().to_owned(),
                reason: unmet(&message.requirement(), "message"),
            }));
        }

        match message {
            // Saying hello is a deliberate act by a person who has just opened a tab, so it
            // is one of the things the 24-hour window is measured from (v1 §2).
            Incoming::Hello => {
                self.note_a_deliberate_act().await?;
                self.lobby_document().await.map(Some)
            }
        }
    }

    /// The lobby again, if it has moved since the last time this socket saw it.
    ///
    /// The push carries the lobby document, which is an operation like any other and is
    /// `SignedIn` (`docs/spec/api-surface.md`). Checking it here is what closes a socket
    /// whose sign-in has ended, been locked out or been signed out from another tab —
    /// within a tick, rather than whenever the client next says something.
    async fn pushed(&mut self) -> Result<Option<Outgoing>, StoreError> {
        if !self.permitted(&Requirement::SignedIn).await? {
            return Ok(Some(Outgoing::Closing {
                reason: "That sign-in has ended.".to_owned(),
            }));
        }

        // Nothing has been sent yet, so there is nothing to be a change from: the client
        // asked for none of this and the socket waits to be greeted.
        if self.sent.is_none() {
            return Ok(None);
        }

        let lobby = self.lobby().await?;
        if self.already_sent(&lobby) {
            return Ok(None);
        }

        Ok(Some(self.versioned(lobby)))
    }

    /// The lobby as it stands, whether or not it has changed.
    async fn lobby_document(&mut self) -> Result<Outgoing, StoreError> {
        let lobby = self.lobby().await?;

        Ok(match self.already_sent(&lobby) {
            // The same document under the same version. A client that asks twice is told the
            // same thing twice, and a version that moved for a redundant send would make the
            // number mean *how often you asked* rather than *what is true*.
            true => Outgoing::Lobby {
                version: self.version,
                lobby,
            },
            false => self.versioned(lobby),
        })
    }

    /// Whether this is the document this socket already has.
    ///
    /// The version moves when the document does and not otherwise, so *has anything changed*
    /// is asked in one place and answered by comparing the whole document — every field of
    /// it is something the server has committed to keeping true, so any of them differing is
    /// a change.
    fn already_sent(&self, lobby: &Lobby) -> bool {
        self.sent.as_ref() == Some(lobby)
    }

    /// Stamp a changed document with the next version, and remember it.
    fn versioned(&mut self, lobby: Lobby) -> Outgoing {
        self.version += 1;
        self.sent = Some(lobby.clone());

        Outgoing::Lobby {
            version: self.version,
            lobby,
        }
    }

    /// Work out the lobby: the roles this user may assume, and who is in each.
    ///
    /// Eligibility is durable and comes from the store; occupancy is live and comes from the
    /// state authority. **They are composed here rather than behind either of them**, and
    /// that is the seam working rather than leaking: the state authority calls nothing and
    /// holds nothing durable ([ADR-0039]), so a lobby assembled inside it would mean giving
    /// it the store. The two sides meet at the top by passing values, which is the same rule
    /// blast radius is answered by.
    ///
    /// The presence document (#37) is a different case and belongs behind the state
    /// authority: it is a projection over live facts alone, and it is scoped to reach rather
    /// than to eligibility.
    ///
    /// [ADR-0039]: ../../../docs/adr/0039-live-state-is-in-process-behind-one-state-authority.md
    async fn lobby(&self) -> Result<Lobby, StoreError> {
        let mut transaction = self.api.store.begin().await?;
        let read = async {
            let eligible_for = transaction
                .the_roles_open_to(&self.user)
                .await?
                .map_or_else(Vec::new, |(_user, roles)| roles);

            let mut seats = Vec::with_capacity(eligible_for.len());
            for role in &eligible_for {
                seats.push(self.seat(&mut transaction, role).await?);
            }

            Ok(Lobby { roles: seats })
        }
        .await;
        transaction.roll_back().await?;

        read
    }

    /// One role and its occupants, named as the store has them now.
    ///
    /// The names are read live rather than snapshotted: this is a document about what is
    /// true at this moment, not a log entry about what was ([ADR-0028] is the other rule and
    /// this is not it).
    ///
    /// [ADR-0028]: ../../../docs/adr/0028-the-audit-log-records-decisions-not-traffic.md
    async fn seat(&self, transaction: &mut Transaction, role: &Role) -> Result<Seat, StoreError> {
        let mut occupants = Vec::new();
        for occupant in self.api.state.occupants_of(&role.id) {
            if let Some(user) = transaction.user(&occupant).await? {
                occupants.push(user.username);
            }
        }

        Ok(Seat {
            id: role.id.as_str().to_owned(),
            name: role.name.clone(),
            max_occupants: role.max_occupants,
            occupants,
        })
    }

    /// Whether this socket may do the thing it is about to do, as the store stands now.
    async fn permitted(&self, requirement: &Requirement) -> Result<bool, StoreError> {
        let outcome = authorisation::evaluate(
            requirement,
            Presented::cookie(Some(self.sign_in.clone())),
            &self.api.store,
        )
        .await?;

        Ok(matches!(outcome, Outcome::Permitted(_)))
    }

    /// Note that the person holding this sign-in did something deliberate.
    ///
    /// The 24-hour window is measured from these, and **nothing the server pushes counts**:
    /// a console left open on a desk has done nothing, which is what the window is for.
    async fn note_a_deliberate_act(&self) -> Result<(), StoreError> {
        let mut transaction = self.api.store.begin().await?;
        transaction.note_a_deliberate_act(&self.sign_in).await?;
        transaction.commit().await?;

        Ok(())
    }

    /// Send one message, and say whether the socket is still there.
    async fn say(&self, socket: &mut WebSocket, said: &Outgoing) -> Result<(), ()> {
        let Ok(text) = serde_json::to_string(said) else {
            tracing::error!(target: module::TRANSPORT, "a message could not be written");
            return Err(());
        };

        socket
            .send(Message::Text(text.into()))
            .await
            .map_err(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::{
        Eligibilities, NewRole, NewUser, RoleId, Roles, Store, a_temporary_store,
    };
    use crate::identity::Identity;
    use crate::state::StateAuthority;
    use std::sync::Arc;

    /// A deployment with one signed-in user, and whatever roles the test asked for.
    struct ALobby {
        _directory: tempfile::TempDir,
        api: Api,
        user: UserId,
        sign_in: SignInToken,
    }

    impl ALobby {
        /// A user eligible for each of `eligible_for`, and a role they are not eligible for
        /// wherever a test needs one to be absent.
        async fn with(eligible_for: &[(&str, Option<u32>)]) -> Self {
            let (directory, store) = a_temporary_store().await;
            let store = Arc::new(store);
            let state = Arc::new(StateAuthority::empty());

            let mut transaction = store.begin().await.expect("a transaction");
            let user = transaction
                .create_user(NewUser {
                    username: "flight".to_owned(),
                    password_hash: None,
                    is_system_administrator: false,
                })
                .await
                .expect("the user to be created");
            let sign_in = transaction
                .open_sign_in(&user)
                .await
                .expect("the sign-in to open");
            for (name, max_occupants) in eligible_for {
                let role = transaction
                    .create_role(NewRole {
                        name: (*name).to_owned(),
                        max_occupants: *max_occupants,
                    })
                    .await
                    .expect("the role to be created");
                transaction
                    .grant_eligibility(&user, &role)
                    .await
                    .expect("the eligibility to be granted");
            }
            transaction.commit().await.expect("the deployment to land");

            Self {
                _directory: directory,
                api: Api {
                    store,
                    state,
                    identity: Identity::local_passwords(),
                    limits: Arc::new(super::super::RateLimits::default()),
                    bootstrap: None,
                },
                user,
                sign_in,
            }
        }

        /// The socket this user's tab would open.
        fn a_socket(&self) -> Conversation {
            Conversation::opened(self.api.clone(), self.user.clone(), self.sign_in.clone())
        }

        async fn role_named(&self, name: &str) -> RoleId {
            let mut transaction = self.api.store.begin().await.expect("a transaction");
            let roles = transaction.roles().await.expect("the roles to be readable");
            transaction.roll_back().await.expect("the read to close");

            roles
                .into_iter()
                .find(|role| role.name == name)
                .map(|role| role.id)
                .expect("a role by that name")
        }

        /// Somebody else, signed in and holding a seat.
        async fn somebody_occupies(&self, role: &RoleId, username: &str) {
            let mut transaction = self.api.store.begin().await.expect("a transaction");
            let occupant = transaction
                .create_user(NewUser {
                    username: username.to_owned(),
                    password_hash: None,
                    is_system_administrator: false,
                })
                .await
                .expect("the user to be created");
            let sign_in = transaction
                .open_sign_in(&occupant)
                .await
                .expect("the sign-in to open");
            transaction.commit().await.expect("the occupant to land");

            self.api.state.a_session_is_held(&sign_in, &occupant, role);
        }

        async fn end_the_sign_in(&self) {
            let mut transaction = self.api.store.begin().await.expect("a transaction");
            transaction
                .end_sign_in(&self.sign_in)
                .await
                .expect("the sign-in to end");
            transaction.commit().await.expect("the sign-out to land");
        }
    }

    const HELLO: &str = r#"{"message":"hello"}"#;

    async fn said(socket: &mut Conversation, message: &str) -> Outgoing {
        socket
            .received(message)
            .await
            .expect("the socket to answer")
            .expect("something said back")
    }

    fn as_json(said: &Outgoing) -> serde_json::Value {
        serde_json::to_value(said).expect("a message that can be written")
    }

    /// The lobby answers one question — *should I assume a role, and which?* — so what a
    /// socket is greeted with is the roles this user may assume.
    #[tokio::test]
    async fn greets_a_tab_with_the_roles_its_user_is_eligible_for() {
        let lobby = ALobby::with(&[("Flight Director", Some(1)), ("CAPCOM", None)]).await;
        let mut socket = lobby.a_socket();

        let said = said(&mut socket, HELLO).await;

        let Outgoing::Lobby { version, lobby } = &said else {
            panic!("expected the lobby, got {said:?}");
        };
        assert_eq!(*version, 1);
        // `Observer` is seeded at install and every user is made eligible for it as their
        // record is created, so it is in everybody's lobby and belongs in this list.
        let named: Vec<&str> = lobby.roles.iter().map(|seat| seat.name.as_str()).collect();
        assert_eq!(named, ["CAPCOM", "Flight Director", "Observer"]);
        assert_eq!(
            lobby
                .roles
                .iter()
                .map(|seat| seat.max_occupants)
                .collect::<Vec<_>>(),
            [None, Some(1), None],
            "a role's limit is what says whether a seat can be shared"
        );
    }

    /// A role somebody is not eligible for is not a seat they may take, so it is not in
    /// their lobby. Eligibility is the whole of what is listed.
    #[tokio::test]
    async fn leaves_out_a_role_this_user_may_not_assume() {
        let lobby = ALobby::with(&[("Flight Director", Some(1))]).await;
        let mut transaction = lobby.api.store.begin().await.expect("a transaction");
        transaction
            .create_role(NewRole {
                name: "Surgeon".to_owned(),
                max_occupants: Some(1),
            })
            .await
            .expect("the role to be created");
        transaction.commit().await.expect("the role to land");
        let mut socket = lobby.a_socket();

        let said = said(&mut socket, HELLO).await;

        let Outgoing::Lobby { lobby, .. } = &said else {
            panic!("expected the lobby, got {said:?}");
        };
        let named: Vec<&str> = lobby.roles.iter().map(|seat| seat.name.as_str()).collect();
        assert_eq!(named, ["Flight Director", "Observer"], "Surgeon was listed");
    }

    /// Occupancy is who has assumed a role and not relinquished it, and the lobby names
    /// them: who is in a seat is the other half of *should I take it*.
    #[tokio::test]
    async fn names_whoever_occupies_each_role_and_nobody_else() {
        let lobby = ALobby::with(&[("Flight Director", Some(1)), ("CAPCOM", None)]).await;
        let flight = lobby.role_named("Flight Director").await;
        lobby.somebody_occupies(&flight, "gene").await;
        let mut socket = lobby.a_socket();

        let said = said(&mut socket, HELLO).await;

        let Outgoing::Lobby { lobby, .. } = &said else {
            panic!("expected the lobby, got {said:?}");
        };
        let occupied: Vec<(&str, Vec<&str>)> = lobby
            .roles
            .iter()
            .map(|seat| {
                (
                    seat.name.as_str(),
                    seat.occupants.iter().map(String::as_str).collect(),
                )
            })
            .collect();
        assert_eq!(
            occupied,
            [
                ("CAPCOM", vec![]),
                ("Flight Director", vec!["gene"]),
                ("Observer", vec![])
            ]
        );
    }

    /// Being signed in is not occupancy (ADR-0005). Everybody in these tests is signed in,
    /// and a seat nobody has assumed is empty.
    #[tokio::test]
    async fn a_signed_in_user_who_has_assumed_nothing_occupies_nothing() {
        let lobby = ALobby::with(&[("Flight Director", Some(1))]).await;
        let mut socket = lobby.a_socket();

        let said = said(&mut socket, HELLO).await;

        let Outgoing::Lobby { lobby, .. } = &said else {
            panic!("expected the lobby, got {said:?}");
        };
        assert!(lobby.roles[0].occupants.is_empty());
    }

    /// The version moves when the document does and not otherwise, or *is this the same
    /// state* — the one question versioning answers — stops being answerable.
    #[tokio::test]
    async fn the_version_moves_only_when_the_document_moves() {
        let lobby = ALobby::with(&[("Flight Director", Some(1))]).await;
        let flight = lobby.role_named("Flight Director").await;
        let mut socket = lobby.a_socket();
        let first = said(&mut socket, HELLO).await;
        assert!(matches!(first, Outgoing::Lobby { version: 1, .. }));

        let again = said(&mut socket, HELLO).await;
        assert!(
            matches!(again, Outgoing::Lobby { version: 1, .. }),
            "the version moved for a document that had not: {again:?}"
        );
        assert!(
            socket
                .pushed()
                .await
                .expect("the socket to answer")
                .is_none(),
            "an unchanged lobby was pushed at a client that already had it"
        );

        lobby.somebody_occupies(&flight, "gene").await;

        let moved = socket
            .pushed()
            .await
            .expect("the socket to answer")
            .expect("the lobby to be pushed once somebody took a seat");
        let Outgoing::Lobby { version, lobby } = &moved else {
            panic!("expected the lobby, got {moved:?}");
        };
        assert_eq!(*version, 2);
        assert_eq!(lobby.roles[0].occupants, ["gene"]);
    }

    /// Nothing is pushed at a socket that has not said hello: the client asked for none of
    /// it yet, and the document is rendered atomically by something that is ready to render.
    #[tokio::test]
    async fn pushes_nothing_before_the_client_has_said_hello() {
        let lobby = ALobby::with(&[("Flight Director", Some(1))]).await;
        let mut socket = lobby.a_socket();

        assert!(
            socket
                .pushed()
                .await
                .expect("the socket to answer")
                .is_none()
        );
    }

    /// **Every message is authorised, not just the upgrade** (ADR-0054). The socket was
    /// opened by a sign-in that is now over, and the next message is refused on the strength
    /// of the store rather than of the upgrade.
    #[tokio::test]
    async fn refuses_a_message_from_a_sign_in_that_has_ended_since_the_upgrade() {
        let lobby = ALobby::with(&[("Flight Director", Some(1))]).await;
        let mut socket = lobby.a_socket();
        assert!(matches!(
            said(&mut socket, HELLO).await,
            Outgoing::Lobby { .. }
        ));

        lobby.end_the_sign_in().await;

        let said = said(&mut socket, HELLO).await;
        assert_eq!(
            said,
            Outgoing::Refused {
                was: "hello".to_owned(),
                reason: "That message is for a signed-in user.".to_owned(),
            }
        );
    }

    /// ...and the push is checked too, so the socket closes within a tick rather than
    /// waiting for a client that may never say anything again.
    #[tokio::test]
    async fn closes_a_socket_whose_sign_in_has_ended() {
        let lobby = ALobby::with(&[("Flight Director", Some(1))]).await;
        let mut socket = lobby.a_socket();
        said(&mut socket, HELLO).await;

        lobby.end_the_sign_in().await;

        let said = socket
            .pushed()
            .await
            .expect("the socket to answer")
            .expect("the socket to say why it is going");
        assert_eq!(
            said,
            Outgoing::Closing {
                reason: "That sign-in has ended.".to_owned(),
            }
        );
    }

    /// A message needing a session, arriving on a lobby-tier socket, is refused by the same
    /// per-message check (ADR-0054). Nothing has assumed a role, so nothing on this socket
    /// meets it.
    #[tokio::test]
    async fn a_lobby_socket_meets_nothing_above_signed_in() {
        let lobby = ALobby::with(&[("Flight Director", Some(1))]).await;
        let socket = lobby.a_socket();

        assert!(
            socket
                .permitted(&Requirement::SignedIn)
                .await
                .expect("an answer")
        );
        for out_of_reach in [
            Requirement::Session,
            Requirement::SystemAdministration,
            Requirement::ServiceToken,
        ] {
            assert!(
                !socket.permitted(&out_of_reach).await.expect("an answer"),
                "a lobby socket met {out_of_reach:?}"
            );
        }
    }

    /// Nothing defaults to open, and that includes a message nobody has ruled on.
    #[tokio::test]
    async fn refuses_a_message_it_has_no_rule_for() {
        let lobby = ALobby::with(&[("Flight Director", Some(1))]).await;
        let mut socket = lobby.a_socket();

        for nonsense in [r#"{"message":"cut"}"#, "{}", "not json at all"] {
            let said = said(&mut socket, nonsense).await;

            assert_eq!(
                said,
                Outgoing::Refused {
                    was: "that message".to_owned(),
                    reason: "VoxLoop has no message by that name.".to_owned(),
                },
                "{nonsense} was answered"
            );
        }
    }

    /// The wire is JSON, and every message says which message it is. The console renders the
    /// lobby atomically from this and nothing else, so the shape is part of the promise.
    #[tokio::test]
    async fn writes_the_lobby_as_one_named_document() {
        let lobby = ALobby::with(&[("Flight Director", Some(1))]).await;
        let flight = lobby.role_named("Flight Director").await;
        lobby.somebody_occupies(&flight, "gene").await;
        let mut socket = lobby.a_socket();

        let said = as_json(&said(&mut socket, HELLO).await);

        assert_eq!(said["message"], "lobby");
        assert_eq!(said["version"], 1);
        assert_eq!(said["roles"][0]["name"], "Flight Director");
        assert_eq!(said["roles"][0]["max_occupants"], 1);
        assert_eq!(said["roles"][0]["occupants"][0], "gene");
        // No audio, no authority, no talking indicators, no loops: the lobby is scoped to
        // the one question it answers (ADR-0023).
        let seat = said["roles"][0].as_object().expect("a seat");
        let mut named: Vec<&String> = seat.keys().collect();
        named.sort();
        assert_eq!(named, ["id", "max_occupants", "name", "occupants"]);
    }

    /// Saying hello is a deliberate act by somebody who has just opened a tab, and the
    /// 24-hour window is measured from those.
    #[tokio::test]
    async fn saying_hello_is_a_deliberate_act() {
        let lobby = ALobby::with(&[("Flight Director", Some(1))]).await;
        let mut socket = lobby.a_socket();
        aged(
            &lobby.api.store,
            &lobby.sign_in,
            Duration::from_secs(25 * 60 * 60),
        )
        .await;

        said(&mut socket, HELLO).await;

        let mut transaction = lobby.api.store.begin().await.expect("a transaction");
        let ended = transaction
            .end_sign_ins_idle_for(Duration::from_secs(24 * 60 * 60), &[])
            .await
            .expect("the sweep to answer");
        transaction.commit().await.expect("the sweep to land");
        assert!(
            ended.is_empty(),
            "a sign-in that had just said hello was reaped as abandoned"
        );
    }

    /// ...and the same tab, greeted and then left alone for longer than the window, is
    /// reaped: what stopped the clock was the act, not the socket being open.
    #[tokio::test]
    async fn a_tab_left_alone_past_the_window_is_still_reaped() {
        let lobby = ALobby::with(&[("Flight Director", Some(1))]).await;
        let mut socket = lobby.a_socket();
        said(&mut socket, HELLO).await;

        aged(
            &lobby.api.store,
            &lobby.sign_in,
            Duration::from_secs(25 * 60 * 60),
        )
        .await;

        let mut transaction = lobby.api.store.begin().await.expect("a transaction");
        let ended = transaction
            .end_sign_ins_idle_for(Duration::from_secs(24 * 60 * 60), &[])
            .await
            .expect("the sweep to answer");
        transaction.commit().await.expect("the sweep to land");
        assert_eq!(ended, vec![lobby.user.clone()]);
    }

    /// Push a sign-in's clock back, the way a day of nobody touching it would.
    async fn aged(store: &Store, sign_in: &SignInToken, by: Duration) {
        let mut transaction = store.begin().await.expect("a transaction");
        transaction
            .a_sign_in_has_been_idle_for(sign_in, by)
            .await
            .expect("the clock to be moved back");
        transaction.commit().await.expect("the clock to land");
    }
}
