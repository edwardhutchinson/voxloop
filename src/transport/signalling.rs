//! The signalling channel: the lobby, assuming a role, and the presence document.
//!
//! **One socket per tab, opened at sign-in**, starting at `SignedIn` (ADR-0054). It is the
//! one channel live state travels on — the media transport carries audio and nothing else
//! ([ADR-0019]) — and it is a **second authorised surface**, checked **per message and not at
//! the upgrade**.
//!
//! Upgrade-time authorisation is the tempting shortcut and it breaks the moment an
//! administrator edits a grid cell mid-shift: the socket is already open, and a revoked
//! `emit` would keep arming until the operator happened to reconnect. So every message
//! carries a requirement and every requirement is evaluated against the store and the state
//! authority as they stand at that moment, which is the same rule HTTP routes hold and the
//! same evaluator behind it.
//!
//! **The upgrade refuses a service token.** It is registered `SignedIn`, which no token can
//! satisfy, and a request presenting a cookie and a token together is refused rather than
//! resolved by precedence — a service principal has no session, no client and no media path
//! ([ADR-0029]).
//!
//! **A socket is at one of two tiers and moves between them by an act.** It opens in the
//! **lobby**: read-only, no audio, no authority, answering one question — *should I assume a
//! role, and which?* ([ADR-0023]). Assuming a role mints the session, moves the socket to
//! `Session`, and swaps the document it is sent for the **presence document**. Relinquishing
//! moves it back. Nothing infers the tier: a socket is in a session because somebody assumed
//! a role on it, and never because it has been open a while.
//!
//! **Changing role is a relinquish followed by an assume**, and neither the server nor the
//! console dresses that as a transition. Audio genuinely stops, and the socket is told the
//! session ended before it is told what the lobby holds.
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
    AuditEntry, AuditEvent, AuditLog, Eligibilities, Grid, Occupancy, Permission, Role, RoleId,
    Roles, SignInToken, SignIns, StoreError, Transaction, UserId, Users,
};
use crate::state::{Assuming, Ended, InReach, Relinquished, SessionId};
use crate::telemetry::module;

/// How often the lobby is worked out again and pushed if it has moved.
///
/// Slower than the presence document's tick on purpose: everything in the lobby changes at
/// human speed — somebody takes a seat, an administrator grants an eligibility — and there
/// is no audio and no authority riding on it. The document is only sent when it differs from
/// the one before, so a quiet deployment sends nothing at all.
///
/// It is also the tick the **sign-in** is checked on, whichever tier the socket is at: a
/// sign-in that has ended is a socket that must close, and a second a day is a fair price
/// for not asking the store five times a second per session.
const LOBBY_TICK: Duration = Duration::from_secs(1);

/// How often the presence document is worked out again and pushed if it has moved.
///
/// **~5 Hz** (v1 §6). It is the rate the console's own state moves at — a subscription
/// dropped, an arm cleared, an audience recomputed — and it is slow enough that a document
/// is a document rather than a stream.
///
/// The wire format v1 §6 asks for is JSON at this rate under `permessage-deflate` **with
/// context takeover**, so that consecutive near-identical documents cost a delta. The
/// compression half is **not built**: the WebSocket implementation underneath this
/// (`tungstenite`, by way of axum) negotiates no extensions at all, so there is nowhere to
/// ask for one. It is a bandwidth property rather than a correctness one — nothing above
/// this line changes when it lands — and it is tracked as #78 rather than pretended into
/// place here.
const PRESENCE_TICK: Duration = Duration::from_millis(200);

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
    /// The session this socket has assumed a role into, where it has one.
    ///
    /// It is what moves the socket from `SignedIn` to `Session` (ADR-0054), and it is
    /// presented to the evaluator on every message rather than remembered as a tier: a
    /// session ended from another machine takes the tier with it, within a message.
    session: Option<SessionId>,
    /// The version of the presence document this socket has been sent, where it has been
    /// sent one. The number itself belongs to the session ([ADR-0019]); this is only what
    /// tells a change from a redundant send.
    ///
    /// [ADR-0019]: ../../../docs/adr/0019-presence-is-one-versioned-document-scoped-to-reach.md
    sent_presence: Option<u64>,
    /// Monotonic per socket, and it moves only when the lobby does.
    ///
    /// The lobby's version is the socket's own because **there is no session to hold one**:
    /// nobody has assumed anything, so there is nothing for a version to survive a
    /// reconnection on behalf of. A presence document's version is the session's, for
    /// exactly the opposite reason.
    lobby_version: u64,
    /// The last lobby this socket sent, to tell a change from a redundant send.
    sent_lobby: Option<Lobby>,
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
    /// The server answers with whatever document this socket's tier calls for rather than
    /// pushing one at a socket that may not be listening yet. It performs the two rows
    /// `docs/spec/api-surface.md` gives this tier — opening the channel, and the lobby
    /// document it carries — rather than being a third operation of its own.
    ///
    /// #50 extends it to present a session id, which is what makes it the *resume a session*
    /// row too.
    Hello,
    /// Take up a role, creating the session that carries voice.
    ///
    /// `SignedIn` **and eligibility** (`docs/spec/api-surface.md`): the requirement is about
    /// who is asking and the eligibility is about the seat, and the second is checked in the
    /// handler because it names a role the caller supplied — the same reason `Grid` is built
    /// per message rather than registered once.
    Assume { role: String },
    /// Give the role up, ending the session and returning to the lobby.
    ///
    /// It is a full stop rather than a transition (v1 §2). Nothing survives it, and the
    /// socket is told the session ended before it is told what the lobby holds.
    Relinquish,
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
            Self::Hello | Self::Assume { .. } => Requirement::SignedIn,
            Self::Relinquish => Requirement::Session,
        }
    }

    /// The name for a refusal to say back, so an operator is told which message it was about.
    fn named(&self) -> &'static str {
        match self {
            Self::Hello => "hello",
            Self::Assume { .. } => "assume",
            Self::Relinquish => "relinquish",
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
    /// The presence document, whole, for the session this socket holds. Rendered atomically
    /// for the same reason and never merged either.
    Presence {
        version: u64,
        #[serde(flatten)]
        presence: Presence,
    },
    /// The session is over, and why.
    ///
    /// It is said before the lobby that follows it, because *your session ended* and *here
    /// is the lobby* are two facts and the second one alone reads as a console that lost its
    /// place. A user who is merely displaced would otherwise have to guess (v1 §2).
    SessionEnded { reason: String },
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

/// The presence document, as the console renders it.
///
/// **The document is the API** ([ADR-0019]): whatever the console renders must be in here,
/// and anything in here is something the server has committed to keeping true. It is
/// **scoped to reach** — the loops are the ones this session's role holds at least `monitor`
/// on, and the loops it does not are not named, counted or hinted at.
///
/// **Occupancy is deliberately absent**, and that is the one thing scoped differently
/// ([ADR-0048]): the hail picker fetches a roster when it opens rather than riding along at
/// this document's tick rate.
///
/// Subscriptions (#39), arms (#41), staffing state (#48), loop health (#46) and the audience
/// (#49) land in here one ticket at a time.
///
/// [ADR-0019]: ../../../docs/adr/0019-presence-is-one-versioned-document-scoped-to-reach.md
/// [ADR-0048]: ../../../docs/adr/0048-the-hail-picker-is-the-only-place-the-console-names-a-person.md
#[derive(Serialize, Debug, Clone, PartialEq)]
struct Presence {
    /// The name of the session this document is about. The client keeps it and presents it
    /// on a hello that is resuming (#50); it is not a credential ([ADR-0041]).
    ///
    /// [ADR-0041]: ../../../docs/adr/0041-a-session-is-resumed-by-name.md
    session: String,
    /// The role this session is bound to. Exactly one, always: reach is never composed
    /// across roles and authority never belongs to the person (v1 §1).
    role: AssumedRole,
    loops: Vec<Reachable>,
}

/// The role a session is bound to, named so a console can say what it is.
#[derive(Serialize, Debug, Clone, PartialEq)]
struct AssumedRole {
    id: String,
    name: String,
}

/// One loop this session's role may monitor.
#[derive(Serialize, Debug, Clone, PartialEq)]
struct Reachable {
    id: String,
    name: String,
    /// What the role holds on it — at least `monitor`, or the loop would not be here. The
    /// console needs it to know which loops it may ever speak on, and the document is the
    /// only place it may learn that from.
    permission: &'static str,
}

impl Conversation {
    fn opened(api: Api, user: UserId, sign_in: SignInToken) -> Self {
        Self {
            api,
            user,
            sign_in,
            session: None,
            sent_presence: None,
            lobby_version: 0,
            sent_lobby: None,
        }
    }

    /// Carry one socket until it goes away, or until the sign-in behind it does.
    async fn talk(mut self, mut socket: WebSocket) {
        let mut lobby = tokio::time::interval(LOBBY_TICK);
        lobby.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut presence = tokio::time::interval(PRESENCE_TICK);
        presence.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            let said = tokio::select! {
                received = socket.recv() => match received {
                    // The tab was closed, or the channel was lost. Neither ends anything a
                    // signed-in user holds: a sign-in survives, and so does a session for
                    // its reconnection window (#50).
                    None | Some(Err(_)) => break,
                    Some(Ok(Message::Text(said))) => self.received(&said).await,
                    // Ping, pong, binary and close: nothing VoxLoop says anything in.
                    Some(Ok(_)) => Ok(Vec::new()),
                },
                _ = lobby.tick() => self.pushed_lobby().await,
                _ = presence.tick() => self.pushed_presence().await,
            };

            let said = match said {
                Ok(said) => said,
                Err(error) => {
                    tracing::error!(target: module::TRANSPORT, %error, "the socket could not be answered");
                    vec![Outgoing::Closing {
                        reason: "VoxLoop could not answer that just now.".to_owned(),
                    }]
                }
            };

            let mut going = false;
            for one in &said {
                let closing = matches!(one, Outgoing::Closing { .. });
                if self.say(&mut socket, one).await.is_err() || closing {
                    going = true;
                    break;
                }
            }
            if going {
                break;
            }
        }
    }

    /// Answer one message from the client.
    ///
    /// The requirement is evaluated **now**, against the store and the state authority, for
    /// this message. A message needing a session arriving on a lobby-tier socket is refused
    /// by this same check, and so is one from a sign-in that ended a second ago.
    async fn received(&mut self, said: &str) -> Result<Vec<Outgoing>, StoreError> {
        let Ok(message) = serde_json::from_str::<Incoming>(said) else {
            // Nothing defaults to open, and that includes a message nobody has ruled on:
            // the socket does not guess what an unknown name meant.
            return Ok(vec![Outgoing::Refused {
                was: "that message".to_owned(),
                reason: "VoxLoop has no message by that name.".to_owned(),
            }]);
        };

        if !self.permitted(&message.requirement()).await? {
            return Ok(vec![Outgoing::Refused {
                was: message.named().to_owned(),
                reason: unmet(&message.requirement(), "message"),
            }]);
        }

        // Every one of these is a deliberate act by a person in front of a console, so each
        // is one of the things the 24-hour window is measured from (v1 §2). Nothing the
        // server pushes counts, which is why this is here rather than on the tick.
        self.note_a_deliberate_act().await?;

        match message {
            Incoming::Hello => self.whatever_this_socket_renders().await,
            Incoming::Assume { role } => self.assume(RoleId::presented(role)).await,
            Incoming::Relinquish => self.relinquish().await,
        }
    }

    /// The document this socket's tier calls for, whether or not it has changed.
    ///
    /// A client that asks twice is told the same thing twice, under the same version: a
    /// version that moved for a redundant send would make the number mean *how often you
    /// asked* rather than *what is true*.
    async fn whatever_this_socket_renders(&mut self) -> Result<Vec<Outgoing>, StoreError> {
        if self.session.is_some() {
            return self.presence(Told::WhetherOrNotItMoved).await;
        }

        Ok(vec![self.the_lobby().await?])
    }

    /// The lobby as it stands, stamped with the version it has earned.
    ///
    /// It is always worth sending — every caller of this is answering something the client
    /// asked for, or putting a console back in the lobby it has just landed in — and the
    /// version moves only where the document has.
    async fn the_lobby(&mut self) -> Result<Outgoing, StoreError> {
        let lobby = self.lobby().await?;

        Ok(match self.already_sent(&lobby) {
            true => Outgoing::Lobby {
                version: self.lobby_version,
                lobby,
            },
            false => self.versioned(lobby),
        })
    }

    /// Take up a role.
    ///
    /// **Eligibility is the gate and the grid is not.** A user eligible for a role with an
    /// empty row may assume it and reach nothing, which is an ordinary configuration rather
    /// than a contradiction (v1 §1) — so nothing here consults the grid, and the presence
    /// document that follows is simply empty.
    ///
    /// A role that is not there and a role this user may not assume are **one refusal**.
    /// Telling them apart would answer *does this role exist* to somebody with no business
    /// asking, and neither answer changes what they may do.
    async fn assume(&mut self, role: RoleId) -> Result<Vec<Outgoing>, StoreError> {
        let mut transaction = self.api.store.begin().await?;
        let read = async {
            let role = transaction.role(&role).await?;
            let eligible = match &role {
                Some(role) => transaction.is_eligible(&self.user, &role.id).await?,
                None => false,
            };

            Ok(role.filter(|_| eligible))
        }
        .await;
        transaction.roll_back().await?;

        let Some(role) = read? else {
            return Ok(vec![Outgoing::Refused {
                was: "assume".to_owned(),
                reason: "That is not a role you may assume.".to_owned(),
            }]);
        };

        // The limit is Configuration's and the seat is the state authority's, and they meet
        // here by passing a value ([ADR-0039]) — the same way the blast radius does.
        let assumed = self.api.state.assume(Assuming {
            sign_in: self.sign_in.clone(),
            occupant: self.user.clone(),
            role: role.id.clone(),
            limit: role.max_occupants,
        });

        let assumed = match assumed {
            Ok(assumed) => assumed,
            // An occupied single-occupant role is **always refused, never granted silently**
            // (v1 §2). The limit is said out loud, because *somebody is there* and *this
            // seat is not shared* are the two halves of the answer.
            Err(occupied) => {
                return Ok(vec![Outgoing::Refused {
                    was: "assume".to_owned(),
                    reason: format!(
                        "{} is occupied, and it seats {}.",
                        role.name, occupied.limit
                    ),
                }]);
            }
        };

        let mut transaction = self.api.store.begin().await?;
        let recorded = async {
            // The displaced session is recorded first, because that is the order the two
            // things happened in: the seat somebody was in was given up before the new one
            // was taken. Its sign-in is back in the lobby, so its clock starts from now.
            if let Some(displaced) = &assumed.displaced {
                record_the_end_of(&mut transaction, displaced).await?;
                transaction.the_clock_starts_now(&displaced.sign_in).await?;
            }

            let actor_name = super::name_as_it_stands(&mut transaction, &self.user).await?;

            transaction
                .record(AuditEntry {
                    event: AuditEvent::SessionStarted,
                    actor: Some(self.user.clone()),
                    actor_name,
                    // The socket's source is the upgrade's, and the upgrade is not this act.
                    // Where a session started from is #50's to carry, with the resume.
                    source: None,
                    write: None,
                    operation: None,
                    occupancy: Some(Occupancy {
                        role: role.id.clone(),
                        role_name: role.name.clone(),
                        reason: None,
                    }),
                })
                .await
        }
        .await;

        // **A seat is not taken unless the taking was recorded** (v1 §12). The live change
        // had to come first, because the limit can only be enforced by making it — so a
        // store that could not be written to undoes it here rather than leaving it standing.
        // This socket is about to close, and a role occupied by a session nobody is on is a
        // position nobody can take and nobody can free.
        match recorded {
            Ok(()) => transaction.commit().await?,
            Err(error) => {
                transaction.roll_back().await?;
                self.api.state.ended_by_its_own_holder(&assumed.session);

                return Err(error);
            }
        }

        self.session = Some(assumed.session);
        self.sent_presence = None;

        self.presence(Told::WhetherOrNotItMoved).await
    }

    /// Give the role up.
    ///
    /// The session ends, the seat frees, and the socket drops back to the lobby tier. The two
    /// things it is told — *this ended, and here is why* and *here is the lobby* — are sent
    /// in that order and never merged, because a console handed only the second one has been
    /// shown a state change with no account of it.
    async fn relinquish(&mut self) -> Result<Vec<Outgoing>, StoreError> {
        let Some(session) = self.session.take() else {
            // Unreachable: `Session` was met a moment ago, and only this socket clears it.
            return Ok(Vec::new());
        };
        self.sent_presence = None;

        // No tombstone: this socket is the one doing it and is answered directly, so a
        // reason left behind would be a message with no reader.
        let Some(ended) = self.api.state.ended_by_its_own_holder(&session) else {
            // It ended somewhere else between the requirement and here. Whoever ended it
            // recorded why, and the socket is told on its next tick.
            return self.pushed_presence().await;
        };

        // **The session ends whether or not the log can be written.** Refusing to take
        // somebody off the air because the store is unavailable is the wrong way round: they
        // asked to stop, stopping is the safe direction, and an entry that could not be
        // written is a fault to shout about rather than a reason to keep an operator keyed
        // up. The socket closes after this, which is what says the deployment is unwell.
        self.audit_that_it_ended(&ended).await?;

        self.back_to_the_lobby(ended.why.said()).await
    }

    /// The presence document, if this socket should be sent one.
    ///
    /// It is **assembled from both seams and computed by neither alone**: the reach comes
    /// from the grid, which is durable, and is handed to the state authority as a value —
    /// which then projects the document and stamps the version ([ADR-0039]). The reach is
    /// read afresh every time, because a cell edit that grants or revokes `monitor` must
    /// narrow or widen a live session's document mid-session ([ADR-0019]).
    ///
    /// A session that is no longer there is not an error: it is what a displaced console
    /// finds, and what it is owed is the reason and the lobby.
    ///
    /// [ADR-0019]: ../../../docs/adr/0019-presence-is-one-versioned-document-scoped-to-reach.md
    /// [ADR-0039]: ../../../docs/adr/0039-live-state-is-in-process-behind-one-state-authority.md
    async fn presence(&mut self, told: Told) -> Result<Vec<Outgoing>, StoreError> {
        let Some(session) = self.session.clone() else {
            return Ok(Vec::new());
        };

        let Some(role) = self.api.state.the_role_of(&session) else {
            return self.the_session_ended().await;
        };

        let mut transaction = self.api.store.begin().await?;
        let read = async {
            let named = transaction.role(&role).await?;
            let reach = transaction.the_reach_of(&role, Permission::Monitor).await?;

            Ok((named, reach))
        }
        .await;
        transaction.roll_back().await?;
        let (Some(named), reach) = read? else {
            // The role was deleted out from under a live session. Ending it belongs to the
            // blast radius #53 computes; until then the honest answer is that there is
            // nothing to render, and the socket is told the session is over.
            return self.the_session_ended().await;
        };

        let within = reach
            .into_iter()
            .map(|(held_on, permission)| InReach {
                id: held_on.id,
                name: held_on.name,
                permission,
            })
            .collect();

        // The session's role cannot change under it — a re-assume mints a new session — so
        // the name read above is the name of the role this document comes back bound to.
        let Some((version, presence)) = self.api.state.presence(&session, within) else {
            return self.the_session_ended().await;
        };

        if told == Told::OnlyIfItMoved && self.sent_presence == Some(version) {
            return Ok(Vec::new());
        }
        self.sent_presence = Some(version);

        Ok(vec![Outgoing::Presence {
            version,
            presence: Presence {
                session: presence.session.as_str().to_owned(),
                role: AssumedRole {
                    id: named.id.as_str().to_owned(),
                    name: named.name,
                },
                loops: presence
                    .loops
                    .into_iter()
                    .map(|held_on| Reachable {
                        id: held_on.id.as_str().to_owned(),
                        name: held_on.name,
                        permission: held_on.permission.as_str(),
                    })
                    .collect(),
            },
        }])
    }

    /// This socket's session is gone, and somebody else ended it.
    ///
    /// The reason was left where the ending happened, so it is taken and said. Where there is
    /// none to take — an ending older than the tombstone's memory — the honest answer is the
    /// generic one rather than an invented specific.
    async fn the_session_ended(&mut self) -> Result<Vec<Outgoing>, StoreError> {
        let ended = self
            .session
            .take()
            .and_then(|session| self.api.state.why_it_ended(&session));
        self.sent_presence = None;

        self.back_to_the_lobby(ended.map_or("That session has ended.", Ended::said))
            .await
    }

    /// Say that the session ended and why, then what the lobby holds.
    ///
    /// The lobby goes out **whether or not it has moved** — coming back from a session is a
    /// change of what is on screen rather than a change to the lobby, and a socket that saw
    /// this same lobby an hour ago still has to be given it now. Its **version does not move
    /// for that**, because the version is about the document and not about who is looking at
    /// it: bumping it here would make the number mean *how often you were sent this* rather
    /// than *what is true*.
    async fn back_to_the_lobby(&mut self, why: &str) -> Result<Vec<Outgoing>, StoreError> {
        Ok(vec![
            Outgoing::SessionEnded {
                reason: why.to_owned(),
            },
            self.the_lobby().await?,
        ])
    }

    /// The lobby again, if it has moved since the last time this socket saw it.
    ///
    /// The push carries the lobby document, which is an operation like any other and is
    /// `SignedIn` (`docs/spec/api-surface.md`). Checking it here is what closes a socket
    /// whose sign-in has ended, been locked out or been signed out from another tab —
    /// within a tick, rather than whenever the client next says something. It is checked at
    /// both tiers, because a session does not outlive the sign-in it was assumed from.
    async fn pushed_lobby(&mut self) -> Result<Vec<Outgoing>, StoreError> {
        if !self.permitted(&Requirement::SignedIn).await? {
            return Ok(vec![Outgoing::Closing {
                reason: "That sign-in has ended.".to_owned(),
            }]);
        }

        // Nothing has been sent yet, so there is nothing to be a change from: the client
        // asked for none of this and the socket waits to be greeted. A socket in a session
        // is not shown the lobby at all.
        if self.sent_lobby.is_none() || self.session.is_some() {
            return Ok(Vec::new());
        }

        let lobby = self.lobby().await?;
        if self.already_sent(&lobby) {
            return Ok(Vec::new());
        }

        Ok(vec![self.versioned(lobby)])
    }

    /// The presence document again, if it has moved since the last time this session saw it.
    async fn pushed_presence(&mut self) -> Result<Vec<Outgoing>, StoreError> {
        if self.session.is_none() || self.sent_presence.is_none() {
            return Ok(Vec::new());
        }

        self.presence(Told::OnlyIfItMoved).await
    }

    /// Whether this is the lobby this socket already has.
    ///
    /// The version moves when the document does and not otherwise, so *has anything changed*
    /// is asked in one place and answered by comparing the whole document — every field of
    /// it is something the server has committed to keeping true, so any of them differing is
    /// a change.
    fn already_sent(&self, lobby: &Lobby) -> bool {
        self.sent_lobby.as_ref() == Some(lobby)
    }

    /// Stamp a changed lobby with the next version, and remember it.
    fn versioned(&mut self, lobby: Lobby) -> Outgoing {
        self.lobby_version += 1;
        self.sent_lobby = Some(lobby.clone());

        Outgoing::Lobby {
            version: self.lobby_version,
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

    /// Whether this socket may do the thing it is about to do, as both seams stand now.
    ///
    /// The session is presented rather than asserted: the evaluator resolves it against the
    /// state authority and against the sign-in behind it, so a socket cannot hold a tier the
    /// live system no longer agrees it has.
    async fn permitted(&self, requirement: &Requirement) -> Result<bool, StoreError> {
        let mut presented = Presented::cookie(Some(self.sign_in.clone()));
        if let Some(session) = &self.session {
            presented = presented.in_session(session.clone());
        }

        let outcome =
            authorisation::evaluate(requirement, presented, &self.api.store, &self.api.state)
                .await?;

        Ok(matches!(outcome, Outcome::Permitted(_)))
    }

    /// Record a session ending in its own transaction, and start that sign-in's clock.
    ///
    /// The two belong together: an entry saying somebody left the seat and a window that
    /// begins the moment they did are the same fact written on both sides (ADR-0023).
    ///
    /// For a relinquish the clock has already been refreshed a moment earlier, because the
    /// message that asked for it was a deliberate act. It is done here anyway because that is
    /// a coincidence of the one caller: an ending that arrives with no message behind it —
    /// the reconnection window running out (#50), a forced relinquish (#51) — has nothing
    /// else to start it.
    async fn audit_that_it_ended(&self, ended: &Relinquished) -> Result<(), StoreError> {
        let mut transaction = self.api.store.begin().await?;
        let recorded = async {
            record_the_end_of(&mut transaction, ended).await?;
            transaction.the_clock_starts_now(&ended.sign_in).await
        }
        .await;
        match recorded {
            Ok(()) => transaction.commit().await?,
            Err(error) => {
                transaction.roll_back().await?;
                return Err(error);
            }
        }

        Ok(())
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

/// Whether a document is worth sending when it has not moved.
///
/// An answer to something the client asked is sent whatever it says; a push is sent only
/// when there is something new in it, or the tick would be a stream rather than a document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Told {
    WhetherOrNotItMoved,
    OnlyIfItMoved,
}

/// Write the audit entry for a session that ended, **with the reason** (v1 §12).
///
/// It is a free function because the two callers are the session's own socket and the socket
/// that displaced it, and an entry that differed between them would say the ending differed.
async fn record_the_end_of(
    transaction: &mut Transaction,
    ended: &Relinquished,
) -> Result<(), StoreError> {
    let role = transaction.role(&ended.role).await?;
    let actor_name = super::name_as_it_stands(transaction, &ended.occupant).await?;

    transaction
        .record(AuditEntry {
            event: AuditEvent::SessionEnded,
            actor: Some(ended.occupant.clone()),
            actor_name,
            source: None,
            write: None,
            operation: None,
            occupancy: Some(Occupancy {
                role: ended.role.clone(),
                role_name: role.map_or_else(String::new, |role| role.name),
                reason: Some(ended.why.stored().to_owned()),
            }),
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::{
        Eligibilities, Loops, NewRole, NewUser, RecordedEntry, Roles, Store, a_temporary_store,
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

        /// A second tab, on the same sign-in and the same person.
        fn another_tab(&self) -> Conversation {
            self.a_socket()
        }

        /// The same person at a second console, signed in there separately. A user may be
        /// signed in on several machines and still holds at most one session (v1 §2), so the
        /// two sign-ins are what tell *this tab* apart from *this person*.
        async fn another_machine(&self) -> Conversation {
            let mut transaction = self.api.store.begin().await.expect("a transaction");
            let elsewhere = transaction
                .open_sign_in(&self.user)
                .await
                .expect("the sign-in to open");
            transaction.commit().await.expect("the sign-in to land");

            Conversation::opened(self.api.clone(), self.user.clone(), elsewhere)
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

        /// A loop, ruled on, with this role holding `permission` on it.
        ///
        /// The mark is dismissed because an unreviewed loop is `none` on every rung whatever
        /// its cells hold, so a test that skipped it would be testing the mark rather than
        /// the reach.
        async fn a_loop_reachable_by(&self, name: &str, role: &RoleId, permission: Permission) {
            let mut transaction = self.api.store.begin().await.expect("a transaction");
            let held_on = transaction
                .create_loop(name)
                .await
                .expect("the loop to be created");
            transaction
                .set_cell(role, &held_on, permission)
                .await
                .expect("the cell to be set");
            transaction
                .dismiss_unreviewed(&held_on)
                .await
                .expect("the column to be ruled on");
            transaction.commit().await.expect("the loop to land");
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

            let limit = self.limit_of(role).await;
            self.api
                .state
                .assume(Assuming {
                    sign_in,
                    occupant,
                    role: role.clone(),
                    limit,
                })
                .unwrap_or_else(|_| panic!("the seat to be free"));
        }

        async fn limit_of(&self, role: &RoleId) -> Option<u32> {
            let mut transaction = self.api.store.begin().await.expect("a transaction");
            let read = transaction.role(role).await.expect("the role to be there");
            transaction.roll_back().await.expect("the read to close");

            read.expect("a role by that id").max_occupants
        }

        async fn end_the_sign_in(&self) {
            let mut transaction = self.api.store.begin().await.expect("a transaction");
            transaction
                .end_sign_in(&self.sign_in)
                .await
                .expect("the sign-in to end");
            transaction.commit().await.expect("the sign-out to land");
        }

        /// The entries the log holds for one event, oldest first.
        async fn entries_of(&self, event: AuditEvent) -> Vec<RecordedEntry> {
            let mut transaction = self.api.store.begin().await.expect("a transaction");
            let entries = transaction
                .recent_entries(100)
                .await
                .expect("the log to be readable");
            transaction.roll_back().await.expect("the read to close");

            entries
                .into_iter()
                .filter(|entry| entry.event == event)
                .rev()
                .collect()
        }
    }

    const HELLO: &str = r#"{"message":"hello"}"#;
    const RELINQUISH: &str = r#"{"message":"relinquish"}"#;

    fn assuming(role: &RoleId) -> String {
        format!(r#"{{"message":"assume","role":"{}"}}"#, role.as_str())
    }

    /// Everything the socket said back to one message.
    async fn all(socket: &mut Conversation, message: &str) -> Vec<Outgoing> {
        socket
            .received(message)
            .await
            .expect("the socket to answer")
    }

    /// The one thing the socket said back, where there was one.
    async fn said(socket: &mut Conversation, message: &str) -> Outgoing {
        let mut said = all(socket, message).await;
        assert_eq!(said.len(), 1, "expected one message, got {said:?}");

        said.remove(0)
    }

    fn as_json(said: &Outgoing) -> serde_json::Value {
        serde_json::to_value(said).expect("a message that can be written")
    }

    /// The lobby a socket was greeted with, or a panic naming what came instead.
    fn the_lobby(said: &Outgoing) -> &Lobby {
        match said {
            Outgoing::Lobby { lobby, .. } => lobby,
            otherwise => panic!("expected the lobby, got {otherwise:?}"),
        }
    }

    /// The presence document a socket was given, or a panic naming what came instead.
    fn the_presence(said: &Outgoing) -> (u64, &Presence) {
        match said {
            Outgoing::Presence { version, presence } => (*version, presence),
            otherwise => panic!("expected the presence document, got {otherwise:?}"),
        }
    }

    // ---- the lobby --------------------------------------------------------------------

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

        let named: Vec<&str> = the_lobby(&said)
            .roles
            .iter()
            .map(|seat| seat.name.as_str())
            .collect();
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

        let occupied: Vec<(&str, Vec<&str>)> = the_lobby(&said)
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

        assert!(the_lobby(&said).roles[0].occupants.is_empty());
    }

    /// The version moves when the document does and not otherwise, or *is this the same
    /// state* — the one question versioning answers — stops being answerable.
    #[tokio::test]
    async fn the_lobby_version_moves_only_when_the_lobby_moves() {
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
                .pushed_lobby()
                .await
                .expect("the socket to answer")
                .is_empty(),
            "an unchanged lobby was pushed at a client that already had it"
        );

        lobby.somebody_occupies(&flight, "gene").await;

        let mut moved = socket.pushed_lobby().await.expect("the socket to answer");
        assert_eq!(moved.len(), 1, "expected one document, got {moved:?}");
        let moved = moved.remove(0);
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
                .pushed_lobby()
                .await
                .expect("the socket to answer")
                .is_empty()
        );
        assert!(
            socket
                .pushed_presence()
                .await
                .expect("the socket to answer")
                .is_empty()
        );
    }

    // ---- authorisation, per message ----------------------------------------------------

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
    /// waiting for a client that may never say anything again. It is checked at both tiers:
    /// a session does not outlive the sign-in it was assumed from.
    #[tokio::test]
    async fn closes_a_socket_whose_sign_in_has_ended() {
        let lobby = ALobby::with(&[("Flight Director", Some(1))]).await;
        let flight = lobby.role_named("Flight Director").await;
        let mut socket = lobby.a_socket();
        said(&mut socket, HELLO).await;
        said(&mut socket, &assuming(&flight)).await;

        lobby.end_the_sign_in().await;

        let said = said(&mut socket, "").await;
        assert_eq!(
            said,
            Outgoing::Refused {
                was: "that message".to_owned(),
                reason: "VoxLoop has no message by that name.".to_owned(),
            },
            "an unreadable message is refused before anything else is asked"
        );
        let mut closing = socket.pushed_lobby().await.expect("the socket to answer");
        assert_eq!(
            closing.remove(0),
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

    /// ...and once a role has been assumed it does, because the session is a live fact the
    /// evaluator reads rather than a tier the socket remembers.
    #[tokio::test]
    async fn assuming_a_role_moves_the_socket_to_the_session_tier() {
        let lobby = ALobby::with(&[("Flight Director", Some(1))]).await;
        let flight = lobby.role_named("Flight Director").await;
        let mut socket = lobby.a_socket();

        said(&mut socket, &assuming(&flight)).await;

        assert!(
            socket
                .permitted(&Requirement::Session)
                .await
                .expect("an answer")
        );
        assert!(
            !socket
                .permitted(&Requirement::SystemAdministration)
                .await
                .expect("an answer"),
            "a session is not a route to the admin console"
        );
    }

    /// Relinquishing takes the tier with it. The socket is back in the lobby, and a message
    /// needing a session is refused again.
    #[tokio::test]
    async fn relinquishing_takes_the_session_tier_away() {
        let lobby = ALobby::with(&[("Flight Director", Some(1))]).await;
        let flight = lobby.role_named("Flight Director").await;
        let mut socket = lobby.a_socket();
        said(&mut socket, &assuming(&flight)).await;

        all(&mut socket, RELINQUISH).await;

        assert!(
            !socket
                .permitted(&Requirement::Session)
                .await
                .expect("an answer")
        );
        let refused = said(&mut socket, RELINQUISH).await;
        assert_eq!(
            refused,
            Outgoing::Refused {
                was: "relinquish".to_owned(),
                reason: "That message is for a user who has assumed a role.".to_owned(),
            }
        );
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

    // ---- assume ------------------------------------------------------------------------

    /// Assuming mints the session and answers with the presence document, which is what the
    /// console renders from that moment on.
    #[tokio::test]
    async fn assuming_a_role_answers_with_the_presence_document() {
        let lobby = ALobby::with(&[("Flight Director", Some(1))]).await;
        let flight = lobby.role_named("Flight Director").await;
        let mut socket = lobby.a_socket();

        let said = said(&mut socket, &assuming(&flight)).await;

        let (version, presence) = the_presence(&said);
        assert_eq!(version, 1);
        assert!(!presence.session.is_empty(), "a session with no name");
        assert_eq!(presence.role.name, "Flight Director");
        assert_eq!(presence.role.id, flight.as_str());
    }

    /// ...and the seat is occupied from that moment, which is what everybody else's lobby
    /// says.
    #[tokio::test]
    async fn assuming_a_role_occupies_it() {
        let lobby = ALobby::with(&[("Flight Director", Some(1))]).await;
        let flight = lobby.role_named("Flight Director").await;
        let mut socket = lobby.a_socket();

        said(&mut socket, &assuming(&flight)).await;

        assert_eq!(lobby.api.state.occupants_of(&flight), vec![lobby.user]);
    }

    /// **A role nobody made this user eligible for is refused**, and so is one that is not
    /// there — with the same words, because telling them apart answers *does this role
    /// exist* to somebody with no business asking.
    #[tokio::test]
    async fn refuses_a_role_this_user_may_not_assume() {
        let lobby = ALobby::with(&[("Flight Director", Some(1))]).await;
        let mut transaction = lobby.api.store.begin().await.expect("a transaction");
        let surgeon = transaction
            .create_role(NewRole {
                name: "Surgeon".to_owned(),
                max_occupants: Some(1),
            })
            .await
            .expect("the role to be created");
        transaction.commit().await.expect("the role to land");
        let mut socket = lobby.a_socket();

        let ineligible = said(&mut socket, &assuming(&surgeon)).await;
        let absent = said(
            &mut socket,
            &assuming(&RoleId::presented("no-such-role".to_owned())),
        )
        .await;

        let refusal = Outgoing::Refused {
            was: "assume".to_owned(),
            reason: "That is not a role you may assume.".to_owned(),
        };
        assert_eq!(ineligible, refusal);
        assert_eq!(absent, refusal, "an absent role was answered differently");
        assert!(lobby.api.state.occupants_of(&surgeon).is_empty());
    }

    /// **An occupied single-occupant role is always refused, never granted silently**
    /// (v1 §2), and the refusal says how many the seat holds.
    #[tokio::test]
    async fn refuses_an_occupied_single_occupant_role() {
        let lobby = ALobby::with(&[("Flight Director", Some(1))]).await;
        let flight = lobby.role_named("Flight Director").await;
        lobby.somebody_occupies(&flight, "gene").await;
        let mut socket = lobby.a_socket();

        let said = said(&mut socket, &assuming(&flight)).await;

        assert_eq!(
            said,
            Outgoing::Refused {
                was: "assume".to_owned(),
                reason: "Flight Director is occupied, and it seats 1.".to_owned(),
            }
        );
        assert_eq!(lobby.api.state.occupants_of(&flight).len(), 1);
    }

    /// A role with **no limit** seats everybody, which is what `Observer` is for.
    #[tokio::test]
    async fn a_role_with_no_limit_is_assumed_however_many_are_in_it() {
        let lobby = ALobby::with(&[("CAPCOM", None)]).await;
        let capcom = lobby.role_named("CAPCOM").await;
        lobby.somebody_occupies(&capcom, "gene").await;
        lobby.somebody_occupies(&capcom, "deke").await;
        let mut socket = lobby.a_socket();

        let said = said(&mut socket, &assuming(&capcom)).await;

        assert!(matches!(said, Outgoing::Presence { .. }), "{said:?}");
        assert_eq!(lobby.api.state.occupants_of(&capcom).len(), 3);
    }

    /// **A user has at most one session**, though they may be signed in on several machines.
    /// Assuming on the second tab ends the first, and the first is told why rather than
    /// left with a socket that went quiet (v1 §2).
    #[tokio::test]
    async fn assuming_elsewhere_ends_the_previous_session_and_tells_it_why() {
        let lobby = ALobby::with(&[("Flight Director", Some(1)), ("CAPCOM", None)]).await;
        let flight = lobby.role_named("Flight Director").await;
        let capcom = lobby.role_named("CAPCOM").await;
        let mut first = lobby.a_socket();
        let mut second = lobby.another_tab();
        said(&mut first, &assuming(&flight)).await;

        said(&mut second, &assuming(&capcom)).await;

        let told = first.pushed_presence().await.expect("the socket to answer");
        assert_eq!(
            told[0],
            Outgoing::SessionEnded {
                reason: Ended::AssumedElsewhere.said().to_owned(),
            }
        );
        assert!(
            matches!(told[1], Outgoing::Lobby { .. }),
            "the displaced console was not put back in the lobby: {told:?}"
        );
        assert!(lobby.api.state.occupants_of(&flight).is_empty());
        assert_eq!(lobby.api.state.occupants_of(&capcom), vec![lobby.user]);
    }

    // ---- relinquish --------------------------------------------------------------------

    /// Relinquishing is a full stop, and the console is told that before it is told what the
    /// lobby holds: a page that merely reappeared would be a state change with no account
    /// of it.
    #[tokio::test]
    async fn relinquishing_says_the_session_ended_and_then_shows_the_lobby() {
        let lobby = ALobby::with(&[("Flight Director", Some(1))]).await;
        let flight = lobby.role_named("Flight Director").await;
        let mut socket = lobby.a_socket();
        said(&mut socket, &assuming(&flight)).await;

        let said = all(&mut socket, RELINQUISH).await;

        assert_eq!(
            said[0],
            Outgoing::SessionEnded {
                reason: Ended::Relinquished.said().to_owned(),
            }
        );
        assert!(matches!(said[1], Outgoing::Lobby { .. }), "{said:?}");
        assert!(lobby.api.state.occupants_of(&flight).is_empty());
    }

    /// **Changing role is a relinquish followed by an assume** (v1 §2), and the socket says
    /// so at every step rather than swapping one document for another.
    #[tokio::test]
    async fn changing_role_is_a_relinquish_followed_by_an_assume() {
        let lobby = ALobby::with(&[("Flight Director", Some(1)), ("CAPCOM", None)]).await;
        let flight = lobby.role_named("Flight Director").await;
        let capcom = lobby.role_named("CAPCOM").await;
        let mut socket = lobby.a_socket();
        said(&mut socket, &assuming(&flight)).await;

        let gave_it_up = all(&mut socket, RELINQUISH).await;
        let took_the_other = said(&mut socket, &assuming(&capcom)).await;

        assert!(matches!(gave_it_up[0], Outgoing::SessionEnded { .. }));
        assert!(matches!(gave_it_up[1], Outgoing::Lobby { .. }));
        let (_version, presence) = the_presence(&took_the_other);
        assert_eq!(presence.role.name, "CAPCOM");
        assert!(lobby.api.state.occupants_of(&flight).is_empty());
    }

    // ---- the presence document ---------------------------------------------------------

    /// **The document is scoped to reach**: a session receives presence only for loops its
    /// role holds at least `monitor` on, and the loops it does not are not named, counted or
    /// hinted at (ADR-0019).
    #[tokio::test]
    async fn the_presence_document_carries_only_the_loops_in_reach() {
        let lobby = ALobby::with(&[("Flight Director", Some(1))]).await;
        let flight = lobby.role_named("Flight Director").await;
        let capcom = lobby.role_named("Observer").await;
        lobby
            .a_loop_reachable_by("Air-to-ground", &flight, Permission::Emit)
            .await;
        lobby
            .a_loop_reachable_by("Flight Director", &flight, Permission::Monitor)
            .await;
        lobby
            .a_loop_reachable_by("Surgeon", &capcom, Permission::Control)
            .await;
        let mut socket = lobby.a_socket();

        let said = said(&mut socket, &assuming(&flight)).await;

        let (_version, presence) = the_presence(&said);
        let reached: Vec<(&str, &str)> = presence
            .loops
            .iter()
            .map(|held_on| (held_on.name.as_str(), held_on.permission))
            .collect();
        assert_eq!(
            reached,
            [("Air-to-ground", "emit"), ("Flight Director", "monitor")],
            "a loop outside this role's reach was in the document"
        );
    }

    /// A loop nobody has ruled on is `none` on every rung whatever its cells hold (v1 §3),
    /// so it is out of reach and out of the document — the same answer the evaluator gives.
    #[tokio::test]
    async fn a_loop_nobody_has_ruled_on_is_out_of_reach() {
        let lobby = ALobby::with(&[("Flight Director", Some(1))]).await;
        let flight = lobby.role_named("Flight Director").await;
        let mut transaction = lobby.api.store.begin().await.expect("a transaction");
        let unreviewed = transaction
            .create_loop("Air-to-ground")
            .await
            .expect("the loop to be created");
        transaction
            .set_cell(&flight, &unreviewed, Permission::Control)
            .await
            .expect("the cell to be set");
        transaction.commit().await.expect("the loop to land");
        let mut socket = lobby.a_socket();

        let said = said(&mut socket, &assuming(&flight)).await;

        assert!(
            the_presence(&said).1.loops.is_empty(),
            "an unreviewed loop reached a session's document"
        );
    }

    /// The document narrows and widens **mid-session**: a cell edit changes what a live
    /// session may see, and the version moves when it does (ADR-0019).
    #[tokio::test]
    async fn the_document_follows_a_cell_edit_without_a_re_assume() {
        let lobby = ALobby::with(&[("Flight Director", Some(1))]).await;
        let flight = lobby.role_named("Flight Director").await;
        let mut socket = lobby.a_socket();
        said(&mut socket, &assuming(&flight)).await;
        assert!(
            socket
                .pushed_presence()
                .await
                .expect("the socket to answer")
                .is_empty(),
            "an unchanged document was pushed"
        );

        lobby
            .a_loop_reachable_by("Air-to-ground", &flight, Permission::Monitor)
            .await;

        let mut pushed = socket
            .pushed_presence()
            .await
            .expect("the socket to answer");
        assert_eq!(pushed.len(), 1, "{pushed:?}");
        let widened = pushed.remove(0);
        let (version, presence) = the_presence(&widened);
        assert_eq!(version, 2);
        assert_eq!(presence.loops.len(), 1);
    }

    /// The wire is JSON, and every message says which message it is. The console renders the
    /// document atomically from this and nothing else, so the shape is part of the promise —
    /// including what is **not** in it: occupancy is scoped differently and is fetched when
    /// the hail picker opens (ADR-0048).
    #[tokio::test]
    async fn writes_the_presence_document_as_one_named_document() {
        let lobby = ALobby::with(&[("Flight Director", Some(1))]).await;
        let flight = lobby.role_named("Flight Director").await;
        lobby
            .a_loop_reachable_by("Air-to-ground", &flight, Permission::Emit)
            .await;
        let mut socket = lobby.a_socket();

        let said = as_json(&said(&mut socket, &assuming(&flight)).await);

        assert_eq!(said["message"], "presence");
        assert_eq!(said["version"], 1);
        assert_eq!(said["role"]["name"], "Flight Director");
        assert_eq!(said["loops"][0]["name"], "Air-to-ground");
        assert_eq!(said["loops"][0]["permission"], "emit");
        let mut named: Vec<&String> = said.as_object().expect("a document").keys().collect();
        named.sort();
        assert_eq!(named, ["loops", "message", "role", "session", "version"]);
    }

    /// The wire is JSON for the lobby too, and the lobby is scoped to the one question it
    /// answers: no audio, no authority, no talking indicators, no loops (ADR-0023).
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
        let seat = said["roles"][0].as_object().expect("a seat");
        let mut named: Vec<&String> = seat.keys().collect();
        named.sort();
        assert_eq!(named, ["id", "max_occupants", "name", "occupants"]);
    }

    /// A socket in a session is sent the presence document and not the lobby: it renders one
    /// thing at a time, and two documents describing two different states would be the torn
    /// state the versioning exists to prevent.
    #[tokio::test]
    async fn a_socket_in_a_session_is_greeted_with_the_presence_document() {
        let lobby = ALobby::with(&[("Flight Director", Some(1))]).await;
        let flight = lobby.role_named("Flight Director").await;
        let mut socket = lobby.a_socket();
        said(&mut socket, HELLO).await;
        said(&mut socket, &assuming(&flight)).await;

        let greeted = said(&mut socket, HELLO).await;

        assert!(matches!(greeted, Outgoing::Presence { .. }), "{greeted:?}");
        assert!(
            socket
                .pushed_lobby()
                .await
                .expect("the socket to answer")
                .is_empty(),
            "a socket in a session was pushed the lobby"
        );
    }

    // ---- audit -------------------------------------------------------------------------

    /// **Session start and session end are audited, with the reason** (v1 §12). They are
    /// authentication events: nothing on disk changed, so there is no before, no after and
    /// no blast radius.
    #[tokio::test]
    async fn records_a_session_starting_and_ending_against_the_role() {
        let lobby = ALobby::with(&[("Flight Director", Some(1))]).await;
        let flight = lobby.role_named("Flight Director").await;
        let mut socket = lobby.a_socket();

        said(&mut socket, &assuming(&flight)).await;
        all(&mut socket, RELINQUISH).await;

        let started = lobby.entries_of(AuditEvent::SessionStarted).await;
        assert_eq!(started.len(), 1);
        assert_eq!(started[0].actor.as_ref(), Some(&lobby.user));
        assert_eq!(started[0].actor_name, "flight");
        let occupancy = started[0].occupancy.as_ref().expect("the seat it was of");
        assert_eq!(occupancy.role, flight);
        assert_eq!(occupancy.role_name, "Flight Director");
        assert_eq!(occupancy.reason, None, "a session start needs no reason");
        assert!(
            started[0].write.is_none(),
            "assuming a role was recorded as a configuration change"
        );

        let ended = lobby.entries_of(AuditEvent::SessionEnded).await;
        assert_eq!(ended.len(), 1);
        let occupancy = ended[0].occupancy.as_ref().expect("the seat it was of");
        assert_eq!(occupancy.role_name, "Flight Director");
        assert_eq!(occupancy.reason.as_deref(), Some("relinquished"));
    }

    /// A displaced session ends for a different reason, and the log has to be able to tell
    /// *what ended somebody's shift* apart.
    #[tokio::test]
    async fn records_a_displaced_session_with_the_reason_it_ended() {
        let lobby = ALobby::with(&[("Flight Director", Some(1)), ("CAPCOM", None)]).await;
        let flight = lobby.role_named("Flight Director").await;
        let capcom = lobby.role_named("CAPCOM").await;
        let mut first = lobby.a_socket();
        let mut second = lobby.another_tab();
        said(&mut first, &assuming(&flight)).await;

        said(&mut second, &assuming(&capcom)).await;

        let ended = lobby.entries_of(AuditEvent::SessionEnded).await;
        assert_eq!(ended.len(), 1);
        let occupancy = ended[0].occupancy.as_ref().expect("the seat it was of");
        assert_eq!(occupancy.role_name, "Flight Director");
        assert_eq!(occupancy.reason.as_deref(), Some("assumed_elsewhere"));

        let started = lobby.entries_of(AuditEvent::SessionStarted).await;
        assert_eq!(started.len(), 2, "both seats being taken were recorded");
    }

    /// A refused assume changed nothing, so it is not a session start. Refused reads are not
    /// audited and this is nearer one than a write (v1 §3).
    #[tokio::test]
    async fn a_refused_assume_starts_no_session_and_records_none() {
        let lobby = ALobby::with(&[("Flight Director", Some(1))]).await;
        let flight = lobby.role_named("Flight Director").await;
        lobby.somebody_occupies(&flight, "gene").await;
        let mut socket = lobby.a_socket();

        said(&mut socket, &assuming(&flight)).await;

        assert!(
            lobby
                .entries_of(AuditEvent::SessionStarted)
                .await
                .is_empty()
        );
    }

    // ---- the sign-in clock -------------------------------------------------------------

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

    /// **The clock runs only in the lobby** (v1 §2), and this is the half that is easy to
    /// miss: sparing a sign-in while it holds a session is not enough, because its stamp goes
    /// on ageing underneath it. The moment the session ends, that sign-in is standing in the
    /// lobby with a stamp from before the shift — and the next sweep reaps it.
    ///
    /// The displaced machine is where it bites, because nobody there did anything: this is
    /// an operator who held a role for thirty hours, was displaced from another console, and
    /// must land in the lobby rather than be signed out for it.
    #[tokio::test]
    async fn a_displaced_session_leaves_its_sign_in_a_full_window_in_the_lobby() {
        let lobby = ALobby::with(&[("Flight Director", Some(1)), ("CAPCOM", None)]).await;
        let flight = lobby.role_named("Flight Director").await;
        let capcom = lobby.role_named("CAPCOM").await;
        let mut on_the_air = lobby.a_socket();
        let mut elsewhere = lobby.another_machine().await;
        said(&mut on_the_air, &assuming(&flight)).await;
        aged(
            &lobby.api.store,
            &lobby.sign_in,
            Duration::from_secs(30 * 60 * 60),
        )
        .await;

        said(&mut elsewhere, &assuming(&capcom)).await;

        assert!(
            reaped(&lobby.api.store).await.is_empty(),
            "a displaced console's sign-in was reaped for the time it spent on the air"
        );
    }

    /// Whoever the sweep would end right now, for a window of a day.
    async fn reaped(store: &Store) -> Vec<UserId> {
        let mut transaction = store.begin().await.expect("a transaction");
        let ended = transaction
            .end_sign_ins_idle_for(Duration::from_secs(24 * 60 * 60), &[])
            .await
            .expect("the sweep to answer");
        transaction.commit().await.expect("the sweep to land");

        ended
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
