//! State authority — every live fact about the running system, and the only writer of any
//! of them.
//!
//! Sessions, occupancy, subscriptions, arms, key state, connection state and loop health all
//! live here, in plain structures owned by this process. There is no second store and there
//! is no Redis ([ADR-0039]): a restart genuinely ends every session, because the media
//! plane cannot survive one at any price and occupancy restored without an audio path would
//! be exactly the lie the product exists to avoid. Users stay **signed in** across a restart
//! — that is durable and lives in Configuration — and must assume their role again.
//!
//! Being the single writer is the point rather than a side effect. Presence documents are
//! projections this module computes rather than records it keeps, which is what lets their
//! versions be monotonic and what they show be simultaneously true ([ADR-0019]).
//!
//! **Nothing durable is read here and nothing durable is written.** Whatever a live decision
//! needs from Configuration — how many may occupy a role, which loops a role may monitor —
//! is passed in as a value by whoever is holding both, which is the same way the blast
//! radius crosses ([ADR-0039]). That is what keeps the two seams from knowing about each
//! other.
//!
//! [ADR-0019]: ../../docs/adr/0019-presence-is-one-versioned-document-scoped-to-reach.md
//! [ADR-0039]: ../../docs/adr/0039-live-state-is-in-process-behind-one-state-authority.md

use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::configuration::{LoopId, Permission, RoleId, SignInToken, UserId};
use crate::secrets;

/// How long a session's tombstone is kept after the session ends ([ADR-0041]).
///
/// Long enough that somebody who was displaced mid-shift and comes back to the tab is told
/// *what* happened rather than merely that something did, and short enough that the honest
/// answer after it is the generic one. It is not a credential's lifetime and nothing is
/// authorised by it.
///
/// [ADR-0041]: ../../docs/adr/0041-a-session-is-resumed-by-name.md
const TOMBSTONES_ARE_KEPT_FOR: Duration = Duration::from_secs(15 * 60);

/// The name of a session, minted by the assume that created it.
///
/// **It is not a credential** ([ADR-0041]). It is presented over a channel the sign-in
/// cookie has already authenticated and can only ever select among that user's own sessions,
/// so holding somebody else's buys nothing. It is unguessable all the same, because a name
/// that can be enumerated is a way to ask which sessions exist.
///
/// [ADR-0041]: ../../docs/adr/0041-a-session-is-resumed-by-name.md
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct SessionId(String);

impl SessionId {
    /// Take an id as a client presented it, on a hello that is resuming (#50).
    #[allow(dead_code)]
    pub(crate) fn presented(id: String) -> Self {
        Self(id)
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// A session's standing with the audio transport ([ADR-0042]).
///
/// The second of the three axes any console state has to be read against, and the mirror of
/// connection state rather than a version of it: the two fail independently in both
/// directions, and a session can be told everything while being heard by nobody.
///
/// **The order of these lines is the ladder**, and it is what makes the merge a `max`.
/// `Ord` is derived from declaration order, so moving one of them silently changes which
/// reading wins when the two ends disagree.
///
/// **A session with no media path has no emission path.** `lost` is where emission is
/// withdrawn, and it covers a transport that has failed and a transport nobody has connected
/// to yet alike — both carry no audio, and a console that drew them differently would be
/// making a distinction the operator cannot act on.
///
/// `impaired` exists for the same reason connection state's `unconfirmed` does: a binary
/// reading would flap on every ICE consent hiccup and cut audio for a reroute that heals
/// itself in a second.
///
/// [ADR-0042]: ../../docs/adr/0042-the-media-path-has-its-own-ladder.md
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum MediaPath {
    /// Audio is crossing, or would if there were any.
    Connected,
    /// A transient fault, of the kind that routinely heals itself. Emission stands.
    Impaired,
    /// Emission is withdrawn. It is the default because it is what is true before anybody
    /// has said otherwise: a session that has just been minted has no path yet.
    #[default]
    Lost,
}

impl MediaPath {
    /// The word the presence document carries, and the one the client reports back.
    ///
    /// The client sends these too, so they are one vocabulary rather than two that have to
    /// agree — a ladder whose two ends spelled a rung differently would fail in the direction
    /// nobody tests.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::Impaired => "impaired",
            Self::Lost => "lost",
        }
    }

    /// A rung as a client named it, or nothing where it named something else.
    ///
    /// Nothing defaults here either. A word this server does not know is refused rather than
    /// read as the nearest rung, because guessing would let a client hold emission open by
    /// mistyping.
    pub(crate) fn presented(said: &str) -> Option<Self> {
        match said {
            "connected" => Some(Self::Connected),
            "impaired" => Some(Self::Impaired),
            "lost" => Some(Self::Lost),
            _ => None,
        }
    }

    /// The worse of two readings. **Green needs both ends, red needs one** ([ADR-0042]).
    ///
    /// It is `pub(crate)` because the media plane merges the two halves of the server's own
    /// end — ICE and DTLS — by the same rule, and one rule written twice is one that can
    /// come to disagree with itself.
    ///
    /// [ADR-0042]: ../../docs/adr/0042-the-media-path-has-its-own-ladder.md
    pub(crate) fn pessimistically_with(self, other: Self) -> Self {
        self.max(other)
    }
}

/// Why a session ended.
///
/// A closed set rather than a sentence, because the lobby has to render it, the audit log
/// has to be filtered on it, and a free-text reason is neither ([ADR-0041]). It grows one
/// ticket at a time: the reconnection window running out (#50), a forced relinquish (#51)
/// and a revoked eligibility (#53) are the ones still to come.
///
/// [ADR-0041]: ../../docs/adr/0041-a-session-is-resumed-by-name.md
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ended {
    /// The occupant gave the role up. Audio stops, and that is the whole of what happened.
    Relinquished,
    /// The same user assumed a role somewhere else, and a user has at most one session
    /// (v1 §2). The displaced console is told this rather than left to infer it from a
    /// socket that went quiet.
    AssumedElsewhere,
}

impl Ended {
    /// The word the audit log holds. These strings reach disk, so they are renamed only by a
    /// migration.
    pub(crate) fn stored(self) -> &'static str {
        match self {
            Self::Relinquished => "relinquished",
            Self::AssumedElsewhere => "assumed_elsewhere",
        }
    }

    /// What the console says to whoever was in the seat.
    ///
    /// It never implies the session continued somewhere: changing role is a relinquish
    /// followed by an assume, and a sentence that softened that would be the class of lie
    /// this product exists to avoid (v1 §2).
    pub(crate) fn said(self) -> &'static str {
        match self {
            Self::Relinquished => "You relinquished the role. Audio has stopped.",
            Self::AssumedElsewhere => {
                "You assumed a role on another machine, so this session ended. Audio has stopped."
            }
        }
    }
}

/// A user's single live connection to the voice loops, bound to exactly one role.
///
/// It carries the sign-in it was assumed from, because the two acts have two lifetimes and
/// the outer one has a clock the inner one stops ([ADR-0023]): a sign-in standing in the
/// lobby ends after 24 hours of nothing, and a sign-in holding one of these does not.
///
/// [ADR-0023]: ../../docs/adr/0023-sign-in-is-to-the-application-and-a-role-is-assumed.md
struct Session {
    id: SessionId,
    sign_in: SignInToken,
    occupant: UserId,
    role: RoleId,
    /// The version of the last document this session was given, and the document itself.
    ///
    /// Both live on the session rather than on the socket, because a version is **monotonic
    /// per session and survives reconnection** ([ADR-0019]) — a counter belonging to the
    /// socket would restart at every blip, and *is this the same state* is the one question
    /// versioning answers.
    ///
    /// [ADR-0019]: ../../docs/adr/0019-presence-is-one-versioned-document-scoped-to-reach.md
    version: u64,
    last: Option<Presence>,
    /// The two ends of the media path, kept apart because the merge is pessimistic and a
    /// single merged field would have nowhere to put the reading that is currently losing.
    ///
    /// **The client is the driver and the server is the backstop** ([ADR-0042]). Both start
    /// at `lost`, which is the truth about a session minted a moment ago: the transport is
    /// being built, nobody has connected to it, and no audio can cross it yet.
    ///
    /// [ADR-0042]: ../../docs/adr/0042-the-media-path-has-its-own-ladder.md
    said_by_the_client: MediaPath,
    seen_by_the_server: MediaPath,
    /// The loops this session is monitoring right now.
    ///
    /// **A subscription is live state and ends with the session** (v1 §5). What outlives it
    /// is the memory of the set, which is personalisation and belongs to Configuration; this
    /// is seeded from that memory at assume and is never read back into it.
    ///
    /// It is not narrowed to reach, and that is [ADR-0051]'s rule rather than an oversight:
    /// a loop the role has lost `monitor` on is kept here and left out of the document, so a
    /// revocation that is undone leaves the console where it was.
    ///
    /// [ADR-0051]: ../../docs/adr/0051-personalisation-is-scoped-to-the-smallest-thing-it-is-about.md
    subscriptions: Vec<LoopId>,
    /// The loops this session has selected as destinations for its voice.
    ///
    /// **Independent of the subscription set in both directions** ([ADR-0013]) and a second
    /// list for exactly that reason: a loop may be armed without being monitored and
    /// monitored without being armed, and an arm folded into the set above would make loops
    /// read `staffed` because somebody was *talking at* them.
    ///
    /// **Unlike a subscription it is narrowed to reach destructively**, in
    /// [`Session::take_the_arms_out_of_reach`], and the difference is the difference between
    /// a preference and a route. A subscription outside reach is kept inert so that a
    /// revocation which is undone leaves the console where it was ([ADR-0051]). An arm that
    /// came back the same way would put somebody on the air again with their hand on
    /// nothing, which is the one class of surprise this product exists to prevent.
    ///
    /// [ADR-0013]: ../../docs/adr/0013-arming-is-independent-of-subscription.md
    /// [ADR-0051]: ../../docs/adr/0051-personalisation-is-scoped-to-the-smallest-thing-it-is-about.md
    arms: Vec<LoopId>,
    /// Whether the client says it is transmitting right now.
    ///
    /// **The client keys and the server is told** ([ADR-0008]). It is a signal rather than a
    /// permission: what it may reach was settled when the arms were made, and this says only
    /// whether voice is going. Everything anybody else is shown about it — the talking
    /// indicator, and the talker's own transmitting lamp — is read from here, because the
    /// server is the sole authority for saying that a transmission is happening and a lamp
    /// lit by a button going down would be the console asserting its own state.
    ///
    /// [ADR-0008]: ../../docs/adr/0008-emission-is-armed-by-the-server-and-keyed-by-the-client.md
    keyed: bool,
    /// The reach this session was last projected within, kept so that the fan-out can be
    /// computed without reading anything durable ([ADR-0039]).
    ///
    /// It is Configuration's answer, handed in by [`StateAuthority::presence`] and held
    /// rather than re-asked, because **the audience is a projection over every session at
    /// once** and this module may not read a store to build one. The document is recomputed
    /// on every tick, so what is here is at most one tick old and is refreshed by the very
    /// mechanism that keeps the console honest.
    ///
    /// A session that has never been projected has an empty one, which is the truthful
    /// answer for a seat nobody has been told about yet: it reaches nothing and nothing
    /// reaches it.
    ///
    /// [ADR-0039]: ../../docs/adr/0039-live-state-is-in-process-behind-one-state-authority.md
    reach: Vec<InReach>,
}

impl Session {
    /// What the two ends amount to. Green needs both, red needs one.
    fn media_path(&self) -> MediaPath {
        self.said_by_the_client
            .pessimistically_with(self.seen_by_the_server)
    }

    /// Whether this session may hear that loop: it is in reach, and it is monitored.
    ///
    /// Both halves are needed and neither implies the other. The rung says what this role
    /// may ever hear and the subscription says what it is hearing now (v1 §5), and a
    /// subscription outside reach is kept precisely so that it can be inert ([ADR-0051]).
    ///
    /// [ADR-0051]: ../../docs/adr/0051-personalisation-is-scoped-to-the-smallest-thing-it-is-about.md
    fn hears(&self, held_on: &LoopId) -> bool {
        self.subscriptions.contains(held_on)
            && self.reach.iter().any(|within| &within.id == held_on)
    }

    /// Drop the arms this session's role may no longer emit on.
    ///
    /// **An arm outside reach is taken away rather than left inert**, which is the one place
    /// this module treats an arm and a subscription differently, and the reason is what each
    /// of them is. A subscription is a preference, so [ADR-0051] keeps it: a revocation that
    /// is undone leaves the console where it was. An arm is a route, and one that came back
    /// on its own when a cell was restored would put an operator on the air without their
    /// hand on anything.
    ///
    /// It is done here because this is where reach arrives. The document that says which
    /// loops are armed and the fan-out that carries voice to them are then the same answer,
    /// rather than two that agree until somebody edits a cell.
    ///
    /// [ADR-0051]: ../../docs/adr/0051-personalisation-is-scoped-to-the-smallest-thing-it-is-about.md
    fn take_the_arms_out_of_reach(&mut self) {
        let reach = &self.reach;
        self.arms.retain(|armed| {
            reach
                .iter()
                .any(|within| &within.id == armed && within.permission.carries(Permission::Emit))
        });
    }
}

/// A session that is over, and why.
///
/// Kept for [`TOMBSTONES_ARE_KEPT_FOR`] so a client that was not the one doing the ending is
/// told what happened ([ADR-0041]). It is live state like everything else here, so it does
/// not survive a restart — which is the case the server's instance id covers instead (#50).
///
/// [ADR-0041]: ../../docs/adr/0041-a-session-is-resumed-by-name.md
struct Tombstone {
    session: SessionId,
    occupant: UserId,
    why: Ended,
    at: Instant,
}

/// Everything live, behind one lock so there is one writer.
#[derive(Default)]
struct Live {
    /// One per occupied seat. A user has at most one, though they may be signed in on
    /// several machines (v1 §2).
    sessions: Vec<Session>,
    /// The sessions that have ended recently, and why.
    tombstones: Vec<Tombstone>,
    /// The fan-out as it was last taken away to be executed.
    ///
    /// It is the same device the presence document uses for its version: the answer is
    /// recomputed and compared, so *has anything changed* is decided by looking at the
    /// answer rather than by remembering to say so at every write. A counter bumped by hand
    /// is a counter somebody forgets to bump in the one method that mattered.
    last_routing: Option<Vec<WhoHears>>,
}

/// The single holder of live state, and the only thing that may read or write it.
///
/// It is shared rather than owned: Transport asks it what to render and the sign-in clock
/// asks it who is on shift, and neither reaches the structures behind it.
#[derive(Default)]
pub(crate) struct StateAuthority {
    live: Mutex<Live>,
}

/// A role somebody is about to take up, and everything the live side needs to rule on it.
///
/// `limit` is Configuration's — it is the role's `max_occupants` — and it arrives as a value
/// because this module reads nothing durable ([ADR-0039]). `None` is a role with no limit,
/// which is the limit left unset rather than a third kind of role ([ADR-0068]).
///
/// [ADR-0039]: ../../docs/adr/0039-live-state-is-in-process-behind-one-state-authority.md
/// [ADR-0068]: ../../docs/adr/0068-a-role-with-no-limit-is-the-limit-left-unset.md
pub(crate) struct Assuming {
    pub(crate) sign_in: SignInToken,
    pub(crate) occupant: UserId,
    pub(crate) role: RoleId,
    pub(crate) limit: Option<u32>,
    /// The loops this (user, role) pair last had up, as Configuration remembers them.
    ///
    /// It arrives as a value for the same reason the limit does, and it is what makes a
    /// restart cost an assume rather than a rebuild ([ADR-0050]): every operator has to
    /// assume again after one, and this is what puts their console back rather than leaving
    /// them to reassemble it by hand during whatever incident caused it.
    ///
    /// An empty set is a console with no loops up, which is what `Observer` ships as.
    ///
    /// [ADR-0050]: ../../docs/adr/0050-personalisation-persists-what-is-safe-to-be-stale.md
    pub(crate) subscribed_to: Vec<LoopId>,
}

/// A role taken up: the session it created, and whatever it ended to create it.
pub(crate) struct Assumed {
    pub(crate) session: SessionId,
    /// The session this one displaced, where the same user held one already. A user has at
    /// most one session, so assuming anywhere ends whatever they had — and the console that
    /// had it is owed the reason (v1 §2).
    pub(crate) displaced: Option<Relinquished>,
}

/// A session that has ended, named well enough to audit.
///
/// The role is here because session start and session end are audited against the role that
/// was occupied (v1 §12), and the id alone would leave an entry nobody can read after the
/// process that minted it is gone.
pub(crate) struct Relinquished {
    pub(crate) session: SessionId,
    /// The sign-in the role was assumed from. It is here because **the clock runs only in
    /// the lobby** ([ADR-0023]): a session ending puts that sign-in back in it, and the
    /// window has to start from then rather than from whenever its tab last did something.
    ///
    /// [ADR-0023]: ../../docs/adr/0023-sign-in-is-to-the-application-and-a-role-is-assumed.md
    pub(crate) sign_in: SignInToken,
    pub(crate) occupant: UserId,
    pub(crate) role: RoleId,
    pub(crate) why: Ended,
}

/// Why an assume did not happen.
///
/// One reason, because there is one: the seat is taken and the role's limit says it cannot
/// be shared. Eligibility is Configuration's and is checked before this is ever called.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Occupied {
    /// How many may be in the seat at once, which is what makes the refusal readable.
    pub(crate) limit: u32,
}

/// A loop a session's role may monitor, as Configuration has it.
///
/// It is handed to [`StateAuthority::presence`] rather than read here: the grid is durable
/// and the scoping is the grid's answer, so the live side is given the reach and projects
/// within it ([ADR-0019]).
///
/// [ADR-0019]: ../../docs/adr/0019-presence-is-one-versioned-document-scoped-to-reach.md
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InReach {
    pub(crate) id: LoopId,
    pub(crate) name: String,
    /// What this session's role holds on it — at least `monitor`, or it would not be here.
    ///
    /// The console needs it to know which loops it may ever speak on, and *the document is
    /// the API*: anything the console renders has to be in here ([ADR-0019]).
    ///
    /// [ADR-0019]: ../../docs/adr/0019-presence-is-one-versioned-document-scoped-to-reach.md
    pub(crate) permission: Permission,
}

/// Where a loop stands on one session's console: the reach it sits in, and the live choices
/// made within it.
///
/// The two halves come from two seams and are composed here rather than inside either of
/// them ([ADR-0039]): [`InReach`] is Configuration's, handed in as a value, and everything
/// beside it is this module's. Arms (#41), mute (#44) and staffing state (#48) join the
/// second half one ticket at a time, which is why this is a pair rather than a loop with a
/// flag on it.
///
/// It is deliberately not called a subscription. **A subscription is the live choice to
/// monitor a loop** (`CONTEXT.md`), and this is the loop the choice is about — most of them
/// on most consoles have no subscription at all.
///
/// [ADR-0039]: ../../docs/adr/0039-live-state-is-in-process-behind-one-state-authority.md
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Standing {
    pub(crate) held_on: InReach,
    /// Whether this session is monitoring this loop.
    ///
    /// **Subscription is distinct from permission** (v1 §5): the loop is here because the
    /// role may monitor it, and this says whether it currently is.
    pub(crate) subscribed: bool,
    /// Whether this session has armed this loop as a destination for its voice.
    ///
    /// A third fact beside the other two rather than a value within either, because
    /// **arming is independent of subscription** ([ADR-0013]): armed and unmonitored is
    /// legal, monitored and unarmed is the common case, and neither can be read off the
    /// other. An armed loop this session is not monitoring is a **blind arm**, which the
    /// console names in words (v1 §4).
    ///
    /// [ADR-0013]: ../../docs/adr/0013-arming-is-independent-of-subscription.md
    pub(crate) armed: bool,
    /// Whether somebody is transmitting on this loop right now.
    ///
    /// **It says that the loop is being spoken on and never who** ([ADR-0033]), so it is one
    /// flag and not a list: identical for one talker and for five, and carrying nothing to
    /// attribute a voice with. It counts **every** live session armed and keyed on the loop,
    /// this one included — an operator's own transmission is a fact about the loop like
    /// anybody else's, and it reaches their console the same way it reaches everybody's,
    /// from the server.
    ///
    /// It is true whether or not this session is monitoring the loop, which is what makes it
    /// the compensation v1 §4 requires for arming blind.
    ///
    /// [ADR-0033]: ../../docs/adr/0033-the-console-shows-that-someone-is-talking-never-who.md
    pub(crate) talking: bool,
}

/// One listener, and the loop they hear a talker on.
///
/// It is the state authority's half of [ADR-0063]'s division: **the audience is computed
/// here and executed there**. What crosses into the media plane is this, translated by
/// Transport into a label the media plane cannot ask questions of — no `LoopId` reaches it,
/// and nothing below the seam may narrow or widen what this says.
///
/// A listener appears **once per destination**, so somebody monitoring two of a talker's
/// armed loops is in the list twice. That is not a doubled stream: the downlink is one
/// stream per audible talker ([ADR-0007]) and the media plane collapses the pairs, which it
/// can only do if it is told them — the recording tap is per (talker, destination loop)
/// ([ADR-0009]) and that is the distinction being preserved.
///
/// [ADR-0007]: ../../docs/adr/0007-the-client-emits-one-stream.md
/// [ADR-0009]: ../../docs/adr/0009-recording-taps-plain-rtp-on-loopback.md
/// [ADR-0063]: ../../docs/adr/0063-the-media-plane-executes-routing-it-never-computes-it.md
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Heard {
    pub(crate) listener: SessionId,
    pub(crate) on: LoopId,
}

/// One talker and everyone who hears them, which is the answer the media plane executes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WhoHears {
    pub(crate) talker: SessionId,
    pub(crate) listeners: Vec<Heard>,
}

/// The presence document: everything one session may see, as of one moment.
///
/// It is a **projection** rather than a record. Nothing here is stored and read back — it is
/// computed from the live facts and the reach handed in, which is what lets the whole of it
/// be true at the same instant rather than assembled from several that were each true at
/// some point ([ADR-0019]).
///
/// What it carries today is the session, the role it is bound to, the loops in reach and
/// which of them the session is monitoring. Arms (#41), staffing state (#48), loop health
/// (#46) and the audience (#49) land in it one ticket at a time, and each of them is a field
/// the server has committed to keeping true from the moment it appears.
///
/// **Occupancy is deliberately not in it** ([ADR-0048]): the hail picker's roster is a
/// snapshot fetched when the picker opens, and pushing deployment-wide occupancy at every
/// session's tick rate to serve a modal open for seconds is the wrong trade.
///
/// [ADR-0019]: ../../docs/adr/0019-presence-is-one-versioned-document-scoped-to-reach.md
/// [ADR-0048]: ../../docs/adr/0048-the-hail-picker-is-the-only-place-the-console-names-a-person.md
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Presence {
    pub(crate) session: SessionId,
    pub(crate) role: RoleId,
    /// Where this session stands with the audio transport, both ends merged ([ADR-0042]).
    ///
    /// It is in the document because the document is the API and the transmit bar renders
    /// it: emission has two independent withdrawal conditions, and the bar has to be able to
    /// say **which** one applies, because a lost state channel and a lost audio path are
    /// different problems with different fixes.
    ///
    /// [ADR-0042]: ../../docs/adr/0042-the-media-path-has-its-own-ladder.md
    pub(crate) media_path: MediaPath,
    /// Whether the server has this session down as transmitting.
    ///
    /// **This is the transmitting lamp** ([ADR-0008]). It is in the document because the
    /// document is the only thing the console renders, and that is the whole of the honesty
    /// rule here: the operator's own lamp lights when this field arrives back saying so, and
    /// never when their own button goes down. The round trip is the cost and it is paid
    /// deliberately — audio is already flowing by then, so it is a display latency rather
    /// than an audio one.
    ///
    /// [ADR-0008]: ../../docs/adr/0008-emission-is-armed-by-the-server-and-keyed-by-the-client.md
    pub(crate) keyed: bool,
    pub(crate) loops: Vec<Standing>,
}

impl StateAuthority {
    /// A running system with nobody on it, which is what a restart leaves.
    pub(crate) fn empty() -> Self {
        Self::default()
    }

    /// Take up a role, creating the session that carries voice.
    ///
    /// **Occupancy has exactly one origin** and this is it: never inferred from eligibility,
    /// from being signed in, or from having a socket open ([ADR-0005]).
    ///
    /// Three rules land together, and they land under one lock because each is only true
    /// with respect to the others:
    ///
    /// - **A user has at most one session** (v1 §2), so whatever they held is displaced and
    ///   told why.
    /// - **`max_occupants` is enforced**, and an occupied single-occupant role is refused
    ///   rather than granted silently. The caller's own session does not count towards the
    ///   limit — it is about to be displaced, so counting it would refuse somebody the seat
    ///   they are already in.
    /// - **The limit is checked before anything is ended**, so a refused assume costs the
    ///   caller nothing. Ending first and refusing second would take an operator off the air
    ///   for a seat they never got.
    ///
    /// [ADR-0005]: ../../docs/adr/0005-occupancy-means-listening-not-signed-in.md
    pub(crate) fn assume(&self, assuming: Assuming) -> Result<Assumed, Occupied> {
        self.write(|live| {
            let held_already = live
                .sessions
                .iter()
                .position(|session| session.occupant == assuming.occupant);

            if let Some(limit) = assuming.limit {
                let occupied = live
                    .sessions
                    .iter()
                    .filter(|session| {
                        session.role == assuming.role && session.occupant != assuming.occupant
                    })
                    .count();

                if occupied >= limit as usize {
                    return Err(Occupied { limit });
                }
            }

            // Whatever this user was last told about a session of theirs is spent: they are
            // on the air again, and a tombstone nobody came back for would outlive its only
            // reader. It is dropped **before** the displacement below, so the one thing this
            // act has to explain — the console it is about to take the air from — survives.
            live.tombstones
                .retain(|tombstone| tombstone.occupant != assuming.occupant);

            let displaced = held_already.map(|held| {
                let relinquished = ended(live.sessions.remove(held), Ended::AssumedElsewhere);
                live.remember(&relinquished);

                relinquished
            });

            let session = SessionId(secrets::unguessable());
            live.sessions.push(Session {
                id: session.clone(),
                sign_in: assuming.sign_in,
                occupant: assuming.occupant,
                role: assuming.role,
                version: 0,
                last: None,
                said_by_the_client: MediaPath::default(),
                seen_by_the_server: MediaPath::default(),
                subscriptions: assuming.subscribed_to,
                // **Nothing is armed and nothing is keyed on a seat just taken.** The
                // subscription set is restored because it is remembered personalisation
                // (ADR-0050) and a restart otherwise costs every operator their console by
                // hand; an arm set restored the same way would put somebody on the air the
                // instant they assumed, which is why nothing remembers one.
                arms: Vec::new(),
                keyed: false,
                reach: Vec::new(),
            });

            Ok(Assumed { session, displaced })
        })
    }

    /// Give up a role, ending the session and returning the user to the lobby.
    ///
    /// It is a full stop rather than a transition (v1 §2). Nothing survives it, and nothing
    /// here pretends otherwise — a session that has been relinquished is gone from every
    /// answer this module gives, including its own presence document.
    ///
    /// It leaves **no tombstone**. A tombstone exists to tell somebody what happened to a
    /// session they were holding, and here the only party with an interest is the caller,
    /// who is doing it and is answered directly — so one left behind would be a message with
    /// no reader, sitting until it expired. An ending somebody *else* caused is the other
    /// case, and [`StateAuthority::assume`] is the only one of those today.
    ///
    /// It is also how an assume is taken back where the act it was part of could not be
    /// completed: the session was minted a moment ago, nobody was told about it, and undoing
    /// it is not an ending anybody needs to hear about.
    ///
    /// Nothing where the id names no session: an ending of something already over is not a
    /// second ending.
    pub(crate) fn ended_by_its_own_holder(&self, session: &SessionId) -> Option<Relinquished> {
        self.write(|live| {
            let held = live.sessions.iter().position(|held| &held.id == session)?;

            Some(ended(live.sessions.remove(held), Ended::Relinquished))
        })
    }

    /// Whether this session exists and is this user's, which is the whole of what `Session`
    /// asks ([ADR-0054]).
    ///
    /// It is a live fact and it is read on **every** message rather than at the upgrade, so
    /// a relinquish from another tab is refused within a message rather than within a
    /// reconnection.
    ///
    /// [ADR-0054]: ../../docs/adr/0054-every-operation-declares-its-authorisation.md
    pub(crate) fn is_held_by(&self, session: &SessionId, occupant: &UserId) -> bool {
        self.read(|live| {
            live.sessions
                .iter()
                .any(|held| &held.id == session && &held.occupant == occupant)
        })
    }

    /// The role a session is acting through, which is where every `Grid` check starts.
    ///
    /// Reach is never composed across roles and never read from the person: a session is
    /// bound to exactly one role, and this is that binding (v1 §1).
    pub(crate) fn the_role_of(&self, session: &SessionId) -> Option<RoleId> {
        self.read(|live| {
            live.sessions
                .iter()
                .find(|held| &held.id == session)
                .map(|held| held.role.clone())
        })
    }

    /// Why this session ended, where it ended recently enough to still be said.
    ///
    /// The tombstone is **taken**: it exists to be told to somebody once, and a reason
    /// re-delivered on every tick would put an ended session's banner back on screen after
    /// the operator dismissed it.
    ///
    /// Nothing where the session is still live, and nothing where it ended longer ago than
    /// [`TOMBSTONES_ARE_KEPT_FOR`] — after which the honest answer is the generic one
    /// ([ADR-0041]).
    ///
    /// [ADR-0041]: ../../docs/adr/0041-a-session-is-resumed-by-name.md
    pub(crate) fn why_it_ended(&self, session: &SessionId) -> Option<Ended> {
        self.write(|live| {
            live.forget_the_old_tombstones();

            let kept = live
                .tombstones
                .iter()
                .position(|tombstone| &tombstone.session == session)?;

            Some(live.tombstones.remove(kept).why)
        })
    }

    /// Monitor a loop.
    ///
    /// **The live choice, and distinct from the permission behind it** (v1 §5): the grid
    /// says which loops a role may monitor, and this says which of them it currently is.
    /// The rung was checked before this was called and is not checked here — the live side
    /// reads nothing durable ([ADR-0039]) — so what arrives is an act somebody has already
    /// been found entitled to.
    ///
    /// It is a **set**, so subscribing to a loop already up is the same state rather than a
    /// second subscription. That matters at the console: without optimistic rendering the
    /// card lags the click, and a second click on a card that has not caught up yet must not
    /// undo the first.
    ///
    /// It answers whether a live session took the act, which is the whole of what the caller
    /// needs to know before remembering it. Nothing where the id names no session: an act on
    /// a session that ended under it changes nothing and is worth remembering even less.
    ///
    /// [ADR-0039]: ../../docs/adr/0039-live-state-is-in-process-behind-one-state-authority.md
    pub(crate) fn subscribe(&self, session: &SessionId, to: &LoopId) -> bool {
        self.write(|live| {
            let Some(held) = live.sessions.iter_mut().find(|held| &held.id == session) else {
                return false;
            };

            if !held.subscriptions.contains(to) {
                held.subscriptions.push(to.clone());
            }

            true
        })
    }

    /// Stop monitoring a loop.
    ///
    /// The other half of the toggle, and idempotent for the same reason: dropping a loop
    /// that is already down is the same state.
    ///
    /// **It is not the same act as losing reach.** A loop the role can no longer monitor
    /// stays in the set and out of the document ([ADR-0051]); this is the operator saying
    /// they do not want it, which is the one thing that takes it out.
    ///
    /// [ADR-0051]: ../../docs/adr/0051-personalisation-is-scoped-to-the-smallest-thing-it-is-about.md
    pub(crate) fn unsubscribe(&self, session: &SessionId, from: &LoopId) -> bool {
        self.write(|live| {
            let Some(held) = live.sessions.iter_mut().find(|held| &held.id == session) else {
                return false;
            };

            held.subscriptions.retain(|held_on| held_on != from);

            true
        })
    }

    /// Arm a loop: select it as a destination for this session's voice.
    ///
    /// **`Grid(emit, loop)` was checked before this was called** and is not checked here —
    /// the live side reads nothing durable ([ADR-0039]) — so what arrives is an act somebody
    /// has already been found entitled to. What makes the check load-bearing rather than
    /// advisory is that the fan-out is built from this set and from nothing else: there is no
    /// route to a loop that is not in here, so there is nothing for a client to bypass
    /// ([ADR-0008]).
    ///
    /// **It is not a subscription and never becomes one** ([ADR-0013]). Arming a loop puts
    /// it in no ears, including the arming operator's own — emitting blind is legal, and the
    /// console compensates by naming the blind arms in words rather than by quietly
    /// subscribing on somebody's behalf.
    ///
    /// It is a **set**, like the subscription set and for the same reason: the console does
    /// not render optimistically, so a second click on a control that has not caught up yet
    /// must land on the same state rather than undo the first.
    ///
    /// It answers whether a live session took the act. Nothing where the id names no
    /// session.
    ///
    /// [ADR-0008]: ../../docs/adr/0008-emission-is-armed-by-the-server-and-keyed-by-the-client.md
    /// [ADR-0013]: ../../docs/adr/0013-arming-is-independent-of-subscription.md
    /// [ADR-0039]: ../../docs/adr/0039-live-state-is-in-process-behind-one-state-authority.md
    pub(crate) fn arm(&self, session: &SessionId, to: &LoopId) -> bool {
        self.write(|live| {
            let Some(held) = live.sessions.iter_mut().find(|held| &held.id == session) else {
                return false;
            };

            if !held.arms.contains(to) {
                held.arms.push(to.clone());
            }

            true
        })
    }

    /// Disarm a loop: stop selecting it as a destination.
    ///
    /// The other half of the act, idempotent for the same reason. **Neither half is a
    /// renegotiation** ([ADR-0007]): the client's stream already exists and is unaddressed,
    /// so both directions are a routing change on the server and instant by construction.
    ///
    /// [ADR-0007]: ../../docs/adr/0007-the-client-emits-one-stream.md
    pub(crate) fn disarm(&self, session: &SessionId, from: &LoopId) -> bool {
        self.write(|live| {
            let Some(held) = live.sessions.iter_mut().find(|held| &held.id == session) else {
                return false;
            };

            held.arms.retain(|armed| armed != from);

            true
        })
    }

    /// The client says it is transmitting.
    ///
    /// **Keying is the client's act and this is the signal, not the permission** ([ADR-0008]).
    /// What the transmission may reach was settled at arm time, so there is nothing to check
    /// here and no rung to consult; what this changes is what everybody is *told*, which the
    /// server is the sole authority for.
    ///
    /// The residual is [ADR-0008]'s and is stated rather than papered over: a defective or
    /// hostile client can keep sending audio while claiming to be unkeyed. The arm boundary
    /// caps that to loops the role may already reach, and the media plane's
    /// `AudioLevelObserver` is what makes the discrepancy visible from this end.
    ///
    /// [ADR-0008]: ../../docs/adr/0008-emission-is-armed-by-the-server-and-keyed-by-the-client.md
    pub(crate) fn the_client_keys(&self, session: &SessionId) -> bool {
        self.keying(session, true)
    }

    /// The client says it has stopped transmitting.
    ///
    /// **It is taken at its word and it is not the only thing that stops audio.** A key
    /// state is a claim about a client's own microphone, so this is how a transmission ends
    /// in the ordinary case and never how one is prevented — that is the arm set's job, and
    /// [ADR-0014]'s Cut is the act for taking somebody off the air against their client's
    /// wishes.
    ///
    /// [ADR-0014]: ../../docs/adr/0014-authority-acts-on-emission-are-transient.md
    pub(crate) fn the_client_unkeys(&self, session: &SessionId) -> bool {
        self.keying(session, false)
    }

    /// Whether the server has this session down as transmitting.
    ///
    /// It exists for the corroboration [ADR-0008] requires: audio arriving from a session
    /// that claims to be unkeyed is the discrepancy the `AudioLevelObserver` was turned on
    /// to find, and something above both seams has to be able to ask.
    ///
    /// [ADR-0008]: ../../docs/adr/0008-emission-is-armed-by-the-server-and-keyed-by-the-client.md
    pub(crate) fn is_keyed(&self, session: &SessionId) -> bool {
        self.read(|live| {
            live.sessions
                .iter()
                .any(|held| &held.id == session && held.keyed)
        })
    }

    /// Both directions of the key, in one place so they cannot come to differ.
    fn keying(&self, session: &SessionId, now: bool) -> bool {
        self.write(|live| {
            let Some(held) = live.sessions.iter_mut().find(|held| &held.id == session) else {
                return false;
            };

            held.keyed = now;

            true
        })
    }

    /// The whole fan-out, where it has moved since it was last taken.
    ///
    /// **The audience is computed here and executed there** ([ADR-0063]). Every talker's
    /// audience is worked out together, because one operator taking a loop up changes the
    /// audience of everybody armed on it — an answer scoped to one session would be a
    /// different question, and one nobody could act on.
    ///
    /// **It is per arm rather than per key**, and that is [ADR-0008] rather than an
    /// oversight. Keying is the client muting its own microphone, precisely so that a key
    /// press costs no round trip and no renegotiation; gating the route on the key signal
    /// would put the server back in the latency path and would quietly remove the residual
    /// that ADR explicitly accepts and writes down. So the route stands while a loop is
    /// armed, and voice crosses it while the client is keyed.
    ///
    /// **It moves when it moves**, like the presence document's version, and for the same
    /// reason: it is handed to a sink that takes the whole audience each time, so handing
    /// down an unchanged one would be an instruction to rebuild what is already there.
    /// **Taken rather than read**, because it exists to be executed once — two sockets
    /// asking on the same tick must not both carry it down.
    ///
    /// **It is computed from the reach each session was last projected within**, which is
    /// [`Session::reach`] and is written by [`StateAuthority::presence`]. That is the
    /// ordering to hold onto: a session nobody has asked for a document about reaches nothing
    /// and is reached by nothing, which is the truthful answer for a seat nobody has been
    /// told about yet — and every live session has a socket asking five times a second, so it
    /// is at most one tick old.
    ///
    /// [ADR-0008]: ../../docs/adr/0008-emission-is-armed-by-the-server-and-keyed-by-the-client.md
    /// [ADR-0063]: ../../docs/adr/0063-the-media-plane-executes-routing-it-never-computes-it.md
    pub(crate) fn the_routing_if_it_moved(&self) -> Option<Vec<WhoHears>> {
        self.write(|live| {
            let routing: Vec<WhoHears> = live
                .sessions
                .iter()
                .map(|talker| WhoHears {
                    talker: talker.id.clone(),
                    listeners: live.who_hears(talker),
                })
                .collect();

            if live.last_routing.as_ref() == Some(&routing) {
                return None;
            }
            live.last_routing = Some(routing.clone());

            Some(routing)
        })
    }

    /// The presence document for this session, and the version it carries.
    ///
    /// `within` is the session's **reach** — the loops its role holds at least `monitor` on
    /// — read from the grid by whoever called and handed over as a value. A session receives
    /// presence only for those, and one gate or none: leaking the state of loops a role
    /// cannot touch would erect a second, softer boundary beside the grid that nobody
    /// configured ([ADR-0019]).
    ///
    /// **The version moves when the document moves and not otherwise.** A number that ticked
    /// whether or not anything had changed would make *is this the same state* unanswerable,
    /// which is the one question versioning is for.
    ///
    /// Nothing where the id names no session — which is how a socket learns its session has
    /// ended without being told directly.
    ///
    /// [ADR-0019]: ../../docs/adr/0019-presence-is-one-versioned-document-scoped-to-reach.md
    pub(crate) fn presence(
        &self,
        session: &SessionId,
        within: Vec<InReach>,
    ) -> Option<(u64, Presence)> {
        self.write(|live| {
            // The reach is recorded before anything is projected from it, because two other
            // answers are computed from it for sessions other than this one: who hears this
            // talker, and which loops anybody is talking on. Both are projections over every
            // session at once, and this module may not ask a store for any of it.
            {
                let held = live.sessions.iter_mut().find(|held| &held.id == session)?;
                held.reach = within;
                held.take_the_arms_out_of_reach();
            }

            // Worked out before the session is borrowed again, because it reads every other
            // session: a loop is being spoken on because *somebody* is armed and keyed on
            // it, and who that is never reaches the document ([ADR-0033]).
            let spoken_on = live.the_loops_being_spoken_on();

            let held = live.sessions.iter_mut().find(|held| &held.id == session)?;
            let presence = Presence {
                session: held.id.clone(),
                role: held.role.clone(),
                media_path: held.media_path(),
                keyed: held.keyed,
                // **The narrowing happens here and nowhere else.** The session's set holds
                // whatever it holds; the reach handed in decides what is rendered, so a
                // subscription outside it is inert rather than lost ([ADR-0051]).
                loops: held
                    .reach
                    .iter()
                    .map(|held_on| Standing {
                        subscribed: held.subscriptions.contains(&held_on.id),
                        armed: held.arms.contains(&held_on.id),
                        talking: spoken_on.contains(&held_on.id),
                        held_on: held_on.clone(),
                    })
                    .collect(),
            };

            if held.last.as_ref() != Some(&presence) {
                held.version += 1;
                held.last = Some(presence.clone());
            }

            Some((held.version, presence))
        })
    }

    /// What the client says about its own media path.
    ///
    /// **The client drives this ladder** ([ADR-0042]), and the reason is in the two APIs: a
    /// browser's `RTCPeerConnection` tells a transient `disconnected` from a terminal
    /// `failed`, and mediasoup's server-side `iceState` has no `failed` at all and takes
    /// around thirty seconds of consent freshness to say anything — longer than the whole
    /// signalling ladder. A server-authoritative reading would keep emission live over a dead
    /// audio path for longer than a lost state channel is tolerated.
    ///
    /// It is taken as said and merged rather than trusted outright: this end can be wedged or
    /// lying, and [`StateAuthority::the_server_sees`] is what covers that.
    ///
    /// Nothing where the id names no session, which is what a client reporting into a session
    /// that ended under it finds.
    ///
    /// [ADR-0042]: ../../docs/adr/0042-the-media-path-has-its-own-ladder.md
    pub(crate) fn the_client_says(&self, session: &SessionId, is: MediaPath) {
        self.write(|live| {
            if let Some(held) = live.sessions.iter_mut().find(|held| &held.id == session) {
                held.said_by_the_client = is;
            }
        });
    }

    /// What the server's own end of the media path looks like.
    ///
    /// The **backstop** rather than the driver ([ADR-0042]): it is worse at telling a blip
    /// from a failure and better at the one thing the other end cannot do, which is notice
    /// that the client has stopped telling the truth. The two are merged pessimistically
    /// wherever the document is projected — green needs both, red needs one.
    ///
    /// [ADR-0042]: ../../docs/adr/0042-the-media-path-has-its-own-ladder.md
    pub(crate) fn the_server_sees(&self, session: &SessionId, is: MediaPath) {
        self.write(|live| {
            if let Some(held) = live.sessions.iter_mut().find(|held| &held.id == session) {
                held.seen_by_the_server = is;
            }
        });
    }

    /// Nothing is carrying audio: the worker is gone, and every media path with it.
    ///
    /// A worker's death is not one session's problem and is not recorded as one. It is
    /// **only ever the server's end** that moves — the client is still holding whatever it
    /// last saw, and a browser whose transport has quietly stopped receiving will say so on
    /// its own schedule.
    ///
    /// The sessions themselves stand. A permanently dead media path does not end a session
    /// ([ADR-0042]): the operator is present, reading a working console that can say exactly
    /// what is wrong, and taking the decision off them — possibly mid-fix — is the wrong way
    /// round.
    ///
    /// [ADR-0042]: ../../docs/adr/0042-the-media-path-has-its-own-ladder.md
    pub(crate) fn nothing_is_carried(&self) {
        self.write(|live| {
            for held in &mut live.sessions {
                held.seen_by_the_server = MediaPath::Lost;
            }
        });
    }

    /// Who occupies this role, now.
    ///
    /// Occupancy means a role somebody has assumed and not relinquished — never somebody
    /// merely signed in, and never somebody eligible ([ADR-0005]). An empty answer is
    /// *nobody is in that seat*, which is the answer the lobby exists to give.
    ///
    /// [ADR-0005]: ../../docs/adr/0005-occupancy-means-listening-not-signed-in.md
    pub(crate) fn occupants_of(&self, role: &RoleId) -> Vec<UserId> {
        self.read(|live| {
            live.sessions
                .iter()
                .filter(|session| &session.role == role)
                .map(|session| session.occupant.clone())
                .collect()
        })
    }

    /// The sign-ins that hold a session.
    ///
    /// They are what the 24-hour window spares, because that clock **runs only in the
    /// lobby** ([ADR-0023]). The answer is handed to Configuration as a value, which is the
    /// only way the live side and the durable side ever meet ([ADR-0039]).
    ///
    /// [ADR-0023]: ../../docs/adr/0023-sign-in-is-to-the-application-and-a-role-is-assumed.md
    /// [ADR-0039]: ../../docs/adr/0039-live-state-is-in-process-behind-one-state-authority.md
    pub(crate) fn sign_ins_holding_a_session(&self) -> Vec<SignInToken> {
        self.read(|live| {
            live.sessions
                .iter()
                .map(|session| session.sign_in.clone())
                .collect()
        })
    }

    /// Read live state under the lock.
    ///
    /// A poisoned lock is a panic somewhere else in this module, and the honest answer to
    /// *what is live* after one is *nothing I can vouch for*. Recovering the structures and
    /// carrying on would be reading state a panic left half-written, so this takes the
    /// answer that is true either way.
    fn read<T>(&self, of: impl FnOnce(&Live) -> T) -> T {
        match self.live.lock() {
            Ok(live) => of(&live),
            Err(poisoned) => of(&poisoned.into_inner()),
        }
    }

    /// Write live state under the same lock, for the same reason.
    ///
    /// Every rule that has to hold across two facts at once — the one-session rule and the
    /// occupancy limit — is decided inside one of these, because a check and the write it
    /// justifies taken separately are two moments a second tab can arrive between.
    fn write<T>(&self, of: impl FnOnce(&mut Live) -> T) -> T {
        match self.live.lock() {
            Ok(mut live) => of(&mut live),
            Err(poisoned) => of(&mut poisoned.into_inner()),
        }
    }
}

/// A session, as the thing it becomes the moment it is out of the list.
fn ended(session: Session, why: Ended) -> Relinquished {
    Relinquished {
        session: session.id,
        sign_in: session.sign_in,
        occupant: session.occupant,
        role: session.role,
        why,
    }
}

impl Live {
    /// Who hears this talker, and on which loop.
    ///
    /// The rule is one line and every clause in it is load-bearing: **for each loop the
    /// talker has armed, everybody else monitoring that loop within their own reach**.
    ///
    /// - The arm set is the talker's, already narrowed to the loops their role may emit on,
    ///   so there is no entry to a loop the grid does not permit ([ADR-0008]).
    /// - The subscription is the listener's live choice and their reach is the grid's answer
    ///   about them, and **both** are needed: a subscription outside reach is deliberately
    ///   kept and just as deliberately inert ([ADR-0051]).
    /// - The talker is not in their own audience. Hearing yourself back over the network is
    ///   a fault in an intercom, not a feature.
    ///
    /// **Nothing here asks whether anybody is keyed**, for the reason
    /// [`StateAuthority::the_routing_if_it_moved`] gives.
    ///
    /// [ADR-0008]: ../../docs/adr/0008-emission-is-armed-by-the-server-and-keyed-by-the-client.md
    /// [ADR-0051]: ../../docs/adr/0051-personalisation-is-scoped-to-the-smallest-thing-it-is-about.md
    fn who_hears(&self, talker: &Session) -> Vec<Heard> {
        talker
            .arms
            .iter()
            .flat_map(|armed| {
                self.sessions
                    .iter()
                    .filter(move |listener| listener.id != talker.id && listener.hears(armed))
                    .map(|listener| Heard {
                        listener: listener.id.clone(),
                        on: armed.clone(),
                    })
            })
            .collect()
    }

    /// Every loop somebody is armed and keyed on, deployment-wide.
    ///
    /// It is worked out once per document rather than per loop, and it is deployment-wide
    /// rather than scoped to anybody: a loop is being spoken on or it is not, and which
    /// consoles get to see that is the reach the document is projected within, applied
    /// afterwards. Nothing in here says who, which is [ADR-0033] and the reason this answers
    /// with loops rather than with talkers.
    ///
    /// [ADR-0033]: ../../docs/adr/0033-the-console-shows-that-someone-is-talking-never-who.md
    fn the_loops_being_spoken_on(&self) -> Vec<LoopId> {
        self.sessions
            .iter()
            .filter(|held| held.keyed)
            .flat_map(|held| held.arms.iter().cloned())
            .collect()
    }

    /// Keep a session's ending, so whoever was holding it can be told why.
    fn remember(&mut self, relinquished: &Relinquished) {
        self.forget_the_old_tombstones();
        self.tombstones.push(Tombstone {
            session: relinquished.session.clone(),
            occupant: relinquished.occupant.clone(),
            why: relinquished.why,
            at: Instant::now(),
        });
    }

    /// Drop the tombstones nobody came back for.
    fn forget_the_old_tombstones(&mut self) {
        let now = Instant::now();
        self.tombstones
            .retain(|tombstone| now.duration_since(tombstone.at) < TOMBSTONES_ARE_KEPT_FOR);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::{NewRole, NewUser, Roles, SignIns, Store, Users, a_temporary_store};

    /// A user, a role, and the sign-in the role would be assumed from.
    async fn a_seat(store: &Store, username: &str, role: &str) -> (SignInToken, UserId, RoleId) {
        let mut transaction = store.begin().await.expect("a transaction");
        let user = transaction
            .create_user(NewUser {
                username: username.to_owned(),
                password_hash: None,
                is_system_administrator: false,
            })
            .await
            .expect("the user to be created");
        let sign_in = transaction
            .open_sign_in(&user)
            .await
            .expect("the sign-in to open");
        let role = transaction
            .create_role(NewRole {
                name: role.to_owned(),
                max_occupants: Some(1),
            })
            .await
            .expect("the role to be created");
        transaction.commit().await.expect("the deployment to land");

        (sign_in, user, role)
    }

    /// What a user in front of a console would send: take this role, sharing it with at most
    /// `limit` others.
    fn taking(
        sign_in: &SignInToken,
        occupant: &UserId,
        role: &RoleId,
        limit: Option<u32>,
    ) -> Assuming {
        Assuming {
            sign_in: sign_in.clone(),
            occupant: occupant.clone(),
            role: role.clone(),
            limit,
            // A pair with nothing remembered, which is what a first assume finds and what
            // every test here is about unless it says otherwise.
            subscribed_to: Vec::new(),
        }
    }

    #[tokio::test]
    async fn a_role_nobody_has_assumed_has_no_occupants() {
        let (_directory, store) = a_temporary_store().await;
        let (_sign_in, _user, role) = a_seat(&store, "flight", "Flight Director").await;

        assert!(StateAuthority::empty().occupants_of(&role).is_empty());
    }

    /// Occupancy is per role, so the answer names whoever is in that seat and nobody in any
    /// other one.
    #[tokio::test]
    async fn a_role_answers_whoever_occupies_it_and_nobody_else() {
        let (_directory, store) = a_temporary_store().await;
        let (sign_in, user, role) = a_seat(&store, "flight", "Flight Director").await;
        let (elsewhere, capcom, another) = a_seat(&store, "capcom", "CAPCOM").await;
        let live = StateAuthority::empty();

        live.assume(taking(&sign_in, &user, &role, Some(1)))
            .expect("the seat to be free");
        live.assume(taking(&elsewhere, &capcom, &another, Some(1)))
            .expect("the seat to be free");

        assert_eq!(live.occupants_of(&role), vec![user]);
        assert_eq!(live.occupants_of(&another), vec![capcom]);
    }

    /// The clock runs only in the lobby, and this is the half of that rule the live side
    /// answers: which sign-ins are not standing in it.
    #[tokio::test]
    async fn the_sign_ins_holding_a_session_are_the_ones_that_assumed_a_role() {
        let (_directory, store) = a_temporary_store().await;
        let (sign_in, user, role) = a_seat(&store, "flight", "Flight Director").await;
        let (in_the_lobby, _capcom, _another) = a_seat(&store, "capcom", "CAPCOM").await;
        let live = StateAuthority::empty();
        assert!(live.sign_ins_holding_a_session().is_empty());

        live.assume(taking(&sign_in, &user, &role, Some(1)))
            .expect("the seat to be free");

        let holding = live.sign_ins_holding_a_session();
        assert_eq!(holding.len(), 1);
        assert_eq!(holding[0].as_str(), sign_in.as_str());
        assert_ne!(holding[0].as_str(), in_the_lobby.as_str());
    }

    /// Assuming mints the session that carries voice, and the session is what everything
    /// afterwards is asked about.
    #[tokio::test]
    async fn assuming_mints_a_session_bound_to_the_role() {
        let (_directory, store) = a_temporary_store().await;
        let (sign_in, user, role) = a_seat(&store, "flight", "Flight Director").await;
        let live = StateAuthority::empty();

        let assumed = live
            .assume(taking(&sign_in, &user, &role, Some(1)))
            .expect("the seat to be free");

        assert!(live.is_held_by(&assumed.session, &user));
        assert_eq!(live.the_role_of(&assumed.session), Some(role));
        assert!(assumed.displaced.is_none());
    }

    /// A session belongs to the user who assumed it, and to nobody else. It is not a
    /// credential, but it is not an authority either: the sign-in behind the socket is what
    /// says whose it is.
    #[tokio::test]
    async fn a_session_is_held_by_whoever_assumed_it_and_by_nobody_else() {
        let (_directory, store) = a_temporary_store().await;
        let (sign_in, user, role) = a_seat(&store, "flight", "Flight Director").await;
        let (_elsewhere, somebody, _another) = a_seat(&store, "capcom", "CAPCOM").await;
        let live = StateAuthority::empty();

        let assumed = live
            .assume(taking(&sign_in, &user, &role, Some(1)))
            .expect("the seat to be free");

        assert!(!live.is_held_by(&assumed.session, &somebody));
    }

    /// Relinquishing is a full stop: the seat frees, the session is gone from every answer,
    /// and there is no document left to render.
    #[tokio::test]
    async fn relinquishing_ends_the_session_and_frees_the_seat() {
        let (_directory, store) = a_temporary_store().await;
        let (sign_in, user, role) = a_seat(&store, "flight", "Flight Director").await;
        let live = StateAuthority::empty();
        let assumed = live
            .assume(taking(&sign_in, &user, &role, Some(1)))
            .expect("the seat to be free");

        let ended = live
            .ended_by_its_own_holder(&assumed.session)
            .expect("the session to be there to end");

        assert_eq!(ended.role, role);
        assert_eq!(ended.occupant, user);
        assert!(live.occupants_of(&role).is_empty());
        assert!(!live.is_held_by(&assumed.session, &user));
        assert!(live.the_role_of(&assumed.session).is_none());
        assert!(live.presence(&assumed.session, Vec::new()).is_none());
        assert!(live.sign_ins_holding_a_session().is_empty());
    }

    /// Relinquishing something already over is not a second ending.
    #[tokio::test]
    async fn relinquishing_a_session_that_is_over_ends_nothing() {
        let (_directory, store) = a_temporary_store().await;
        let (sign_in, user, role) = a_seat(&store, "flight", "Flight Director").await;
        let live = StateAuthority::empty();
        let assumed = live
            .assume(taking(&sign_in, &user, &role, Some(1)))
            .expect("the seat to be free");
        live.ended_by_its_own_holder(&assumed.session)
            .expect("the first ending");

        assert!(live.ended_by_its_own_holder(&assumed.session).is_none());
    }

    /// **A user has at most one session**, though they may be signed in on several machines
    /// (v1 §2). Assuming on the second machine ends the first, and says so.
    #[tokio::test]
    async fn a_user_holds_one_session_and_assuming_elsewhere_ends_the_other() {
        let (_directory, store) = a_temporary_store().await;
        let (laptop, user, flight) = a_seat(&store, "flight", "Flight Director").await;
        let (console, _capcom, capcom) = a_seat(&store, "capcom", "CAPCOM").await;
        let live = StateAuthority::empty();
        let first = live
            .assume(taking(&laptop, &user, &flight, Some(1)))
            .expect("the seat to be free");

        let second = live
            .assume(taking(&console, &user, &capcom, Some(1)))
            .expect("the seat to be free");

        let displaced = second.displaced.expect("the first session to be displaced");
        assert_eq!(displaced.session, first.session);
        assert_eq!(displaced.why, Ended::AssumedElsewhere);
        assert!(live.occupants_of(&flight).is_empty());
        assert_eq!(live.occupants_of(&capcom), vec![user]);
        assert_eq!(live.sign_ins_holding_a_session().len(), 1);
    }

    /// ...and the console that lost it is told why rather than left with a socket that went
    /// quiet.
    #[tokio::test]
    async fn a_displaced_session_can_be_told_what_ended_it() {
        let (_directory, store) = a_temporary_store().await;
        let (laptop, user, flight) = a_seat(&store, "flight", "Flight Director").await;
        let (console, _capcom, capcom) = a_seat(&store, "capcom", "CAPCOM").await;
        let live = StateAuthority::empty();
        let first = live
            .assume(taking(&laptop, &user, &flight, Some(1)))
            .expect("the seat to be free");
        live.assume(taking(&console, &user, &capcom, Some(1)))
            .expect("the seat to be free");

        assert_eq!(
            live.why_it_ended(&first.session),
            Some(Ended::AssumedElsewhere)
        );
    }

    /// The reason is told once. A banner that came back on every tick would be one the
    /// operator cannot dismiss.
    #[tokio::test]
    async fn the_reason_a_session_ended_is_said_once() {
        let (_directory, store) = a_temporary_store().await;
        let (laptop, user, flight) = a_seat(&store, "flight", "Flight Director").await;
        let (console, _capcom, capcom) = a_seat(&store, "gene", "CAPCOM").await;
        let live = StateAuthority::empty();
        let displaced = live
            .assume(taking(&laptop, &user, &flight, Some(1)))
            .expect("the seat to be free");
        live.assume(taking(&console, &user, &capcom, Some(1)))
            .expect("the seat to be free");

        assert_eq!(
            live.why_it_ended(&displaced.session),
            Some(Ended::AssumedElsewhere)
        );
        assert_eq!(live.why_it_ended(&displaced.session), None);
    }

    /// **A session its own holder ended leaves nothing behind.** The only party with an
    /// interest was answered directly, so a reason kept for them would be a message with no
    /// reader, sitting until it expired.
    #[tokio::test]
    async fn an_ending_its_own_holder_performed_leaves_no_reason_to_read() {
        let (_directory, store) = a_temporary_store().await;
        let (sign_in, user, role) = a_seat(&store, "flight", "Flight Director").await;
        let live = StateAuthority::empty();
        let assumed = live
            .assume(taking(&sign_in, &user, &role, Some(1)))
            .expect("the seat to be free");

        live.ended_by_its_own_holder(&assumed.session)
            .expect("the session to end");

        assert_eq!(live.why_it_ended(&assumed.session), None);
    }

    /// A session that is still live has not ended, so there is nothing to say about it.
    #[tokio::test]
    async fn a_live_session_has_no_reason_for_ending() {
        let (_directory, store) = a_temporary_store().await;
        let (sign_in, user, role) = a_seat(&store, "flight", "Flight Director").await;
        let live = StateAuthority::empty();
        let assumed = live
            .assume(taking(&sign_in, &user, &role, Some(1)))
            .expect("the seat to be free");

        assert_eq!(live.why_it_ended(&assumed.session), None);
    }

    /// **An occupied single-occupant role is always refused, never granted silently**
    /// (v1 §2). The refusal carries the limit, because *the seat is taken* and *this role
    /// seats one* are the two halves of the answer.
    #[tokio::test]
    async fn refuses_an_occupied_single_occupant_role() {
        let (_directory, store) = a_temporary_store().await;
        let (held, occupant, flight) = a_seat(&store, "gene", "Flight Director").await;
        let (arriving, somebody, _capcom) = a_seat(&store, "flight", "CAPCOM").await;
        let live = StateAuthority::empty();
        live.assume(taking(&held, &occupant, &flight, Some(1)))
            .expect("the seat to be free");

        let refused = live.assume(taking(&arriving, &somebody, &flight, Some(1)));

        assert_eq!(refused.err(), Some(Occupied { limit: 1 }));
        assert_eq!(live.occupants_of(&flight), vec![occupant]);
    }

    /// `max_occupants` is the same concept at every value, so a role seating two takes two
    /// and refuses the third.
    #[tokio::test]
    async fn enforces_max_occupants_above_one() {
        let (_directory, store) = a_temporary_store().await;
        let (first, one, role) = a_seat(&store, "gene", "Support Engineer").await;
        let (second, two, _elsewhere) = a_seat(&store, "flight", "CAPCOM").await;
        let (third, three, _another) = a_seat(&store, "capcom", "Surgeon").await;
        let live = StateAuthority::empty();

        live.assume(taking(&first, &one, &role, Some(2)))
            .expect("the first seat");
        live.assume(taking(&second, &two, &role, Some(2)))
            .expect("the second seat");
        let refused = live.assume(taking(&third, &three, &role, Some(2)));

        assert_eq!(refused.err(), Some(Occupied { limit: 2 }));
        assert_eq!(live.occupants_of(&role).len(), 2);
    }

    /// A role with **no limit** is the limit left unset rather than a third kind of role
    /// ([ADR-0068]), so nothing here refuses anybody.
    #[tokio::test]
    async fn a_role_with_no_limit_seats_everybody() {
        let (_directory, store) = a_temporary_store().await;
        let (first, one, observer) = a_seat(&store, "gene", "Booster").await;
        let (second, two, _elsewhere) = a_seat(&store, "flight", "CAPCOM").await;
        let live = StateAuthority::empty();

        live.assume(taking(&first, &one, &observer, None))
            .expect("no limit to refuse anybody");
        live.assume(taking(&second, &two, &observer, None))
            .expect("no limit to refuse anybody");

        assert_eq!(live.occupants_of(&observer).len(), 2);
    }

    /// The caller's own session does not count towards the limit: it is about to be
    /// displaced. Counting it would refuse somebody the seat they are already in, which is
    /// what a reload from a second tab looks like.
    #[tokio::test]
    async fn re_assuming_the_seat_you_are_already_in_is_not_refused() {
        let (_directory, store) = a_temporary_store().await;
        let (laptop, user, flight) = a_seat(&store, "flight", "Flight Director").await;
        let (console, _elsewhere, _capcom) = a_seat(&store, "gene", "CAPCOM").await;
        let live = StateAuthority::empty();
        let first = live
            .assume(taking(&laptop, &user, &flight, Some(1)))
            .expect("the seat to be free");

        let second = live
            .assume(taking(&console, &user, &flight, Some(1)))
            .expect("the seat this user is already in");

        assert_eq!(
            second.displaced.expect("the first to be displaced").session,
            first.session
        );
        assert_eq!(live.occupants_of(&flight), vec![user]);
    }

    /// **The limit is checked before anything is ended.** A refused assume costs the caller
    /// nothing, or an operator is taken off the air for a seat they never got.
    #[tokio::test]
    async fn a_refused_assume_leaves_the_session_the_caller_already_held() {
        let (_directory, store) = a_temporary_store().await;
        let (held, occupant, flight) = a_seat(&store, "gene", "Flight Director").await;
        let (mine, me, capcom) = a_seat(&store, "flight", "CAPCOM").await;
        let live = StateAuthority::empty();
        live.assume(taking(&held, &occupant, &flight, Some(1)))
            .expect("the seat to be free");
        let standing = live
            .assume(taking(&mine, &me, &capcom, Some(1)))
            .expect("the seat to be free");

        let refused = live.assume(taking(&mine, &me, &flight, Some(1)));

        assert!(refused.is_err());
        assert!(
            live.is_held_by(&standing.session, &me),
            "a refused assume ended the session the caller was holding"
        );
        assert_eq!(live.occupants_of(&capcom), vec![me]);
    }

    /// The presence document is scoped to reach: it carries the loops handed in and nothing
    /// else, because the grid is the one gate on what a session may see (ADR-0019).
    #[tokio::test]
    async fn the_presence_document_carries_the_session_the_role_and_the_reach() {
        let (_directory, store) = a_temporary_store().await;
        let (sign_in, user, role) = a_seat(&store, "flight", "Flight Director").await;
        let live = StateAuthority::empty();
        let assumed = live
            .assume(taking(&sign_in, &user, &role, Some(1)))
            .expect("the seat to be free");

        let (version, presence) = live
            .presence(&assumed.session, vec![a_loop("air-to-ground")])
            .expect("a document for a live session");

        assert_eq!(version, 1);
        assert_eq!(presence.session, assumed.session);
        assert_eq!(presence.role, role);
        assert_eq!(
            presence.loops,
            vec![Standing {
                held_on: a_loop("air-to-ground"),
                // Nothing was remembered and nothing has been taken up, so the loop is on
                // the console, not being heard, not a destination and quiet.
                subscribed: false,
                armed: false,
                talking: false,
            }]
        );
    }

    /// **The version moves when the document moves and not otherwise**, or *is this the same
    /// state* — the one question versioning answers — stops being answerable (ADR-0019).
    #[tokio::test]
    async fn the_version_moves_only_when_the_document_moves() {
        let (_directory, store) = a_temporary_store().await;
        let (sign_in, user, role) = a_seat(&store, "flight", "Flight Director").await;
        let live = StateAuthority::empty();
        let assumed = live
            .assume(taking(&sign_in, &user, &role, Some(1)))
            .expect("the seat to be free");

        let (first, _) = live
            .presence(&assumed.session, vec![a_loop("air-to-ground")])
            .expect("a document");
        let (again, _) = live
            .presence(&assumed.session, vec![a_loop("air-to-ground")])
            .expect("a document");
        let (moved, _) = live
            .presence(
                &assumed.session,
                vec![a_loop("air-to-ground"), a_loop("flight-director")],
            )
            .expect("a document");

        assert_eq!((first, again), (1, 1));
        assert_eq!(moved, 2);
    }

    /// A version belongs to the session, so two sessions count independently and neither
    /// inherits the other's place.
    #[tokio::test]
    async fn versions_are_per_session() {
        let (_directory, store) = a_temporary_store().await;
        let (mine, me, flight) = a_seat(&store, "flight", "Flight Director").await;
        let (theirs, them, capcom) = a_seat(&store, "gene", "CAPCOM").await;
        let live = StateAuthority::empty();
        let one = live
            .assume(taking(&mine, &me, &flight, Some(1)))
            .expect("the seat to be free");
        let two = live
            .assume(taking(&theirs, &them, &capcom, Some(1)))
            .expect("the seat to be free");

        live.presence(&one.session, vec![a_loop("air-to-ground")])
            .expect("a document");
        live.presence(&one.session, Vec::new()).expect("a document");
        let (theirs, _) = live
            .presence(&two.session, vec![a_loop("air-to-ground")])
            .expect("a document");

        assert_eq!(theirs, 1, "one session's version counted the other's");
    }

    // ---- #39: subscription -------------------------------------------------------------

    /// Which loops a document says this session is monitoring, by name.
    fn monitoring(live: &StateAuthority, session: &SessionId, within: Vec<InReach>) -> Vec<String> {
        live.presence(session, within)
            .expect("a live session")
            .1
            .loops
            .into_iter()
            .filter(|held_on| held_on.subscribed)
            .map(|held_on| held_on.held_on.name)
            .collect()
    }

    /// **Subscription is distinct from permission** (v1 §5). A loop in reach is a loop this
    /// role *may* monitor, and a session that has taken nothing up is monitoring nothing.
    #[tokio::test]
    async fn a_loop_in_reach_is_not_a_loop_being_monitored() {
        let (_directory, store) = a_temporary_store().await;
        let (sign_in, user, role) = a_seat(&store, "flight", "Flight Director").await;
        let live = StateAuthority::empty();
        let assumed = live
            .assume(taking(&sign_in, &user, &role, Some(1)))
            .expect("the seat to be free");

        assert!(
            monitoring(&live, &assumed.session, vec![a_loop("air-to-ground")]).is_empty(),
            "a loop was being monitored that nobody took up"
        );
    }

    #[tokio::test]
    async fn subscribing_puts_a_loop_on_the_console_and_unsubscribing_takes_it_off() {
        let (_directory, store) = a_temporary_store().await;
        let (sign_in, user, role) = a_seat(&store, "flight", "Flight Director").await;
        let live = StateAuthority::empty();
        let assumed = live
            .assume(taking(&sign_in, &user, &role, Some(1)))
            .expect("the seat to be free");
        let reach = vec![a_loop("air-to-ground"), a_loop("flight")];

        assert!(live.subscribe(&assumed.session, &a_loop("flight").id));
        assert_eq!(
            monitoring(&live, &assumed.session, reach.clone()),
            ["flight"]
        );

        assert!(live.unsubscribe(&assumed.session, &a_loop("flight").id));
        assert!(monitoring(&live, &assumed.session, reach).is_empty());
    }

    /// The set is a set. Without optimistic rendering the card lags the click, so a second
    /// click on one that has not caught up yet must land on the same state rather than
    /// undoing the first.
    #[tokio::test]
    async fn subscribing_twice_is_one_subscription_and_one_unsubscribe_clears_it() {
        let (_directory, store) = a_temporary_store().await;
        let (sign_in, user, role) = a_seat(&store, "flight", "Flight Director").await;
        let live = StateAuthority::empty();
        let assumed = live
            .assume(taking(&sign_in, &user, &role, Some(1)))
            .expect("the seat to be free");
        let reach = vec![a_loop("flight")];

        live.subscribe(&assumed.session, &a_loop("flight").id);
        live.subscribe(&assumed.session, &a_loop("flight").id);
        live.unsubscribe(&assumed.session, &a_loop("flight").id);

        assert!(monitoring(&live, &assumed.session, reach).is_empty());
    }

    /// The version moves when the document does, and a subscription moves the document.
    #[tokio::test]
    async fn the_version_moves_when_the_subscription_set_does() {
        let (_directory, store) = a_temporary_store().await;
        let (sign_in, user, role) = a_seat(&store, "flight", "Flight Director").await;
        let live = StateAuthority::empty();
        let assumed = live
            .assume(taking(&sign_in, &user, &role, Some(1)))
            .expect("the seat to be free");
        let reach = vec![a_loop("flight")];
        let (first, _) = live
            .presence(&assumed.session, reach.clone())
            .expect("a document");

        live.subscribe(&assumed.session, &a_loop("flight").id);
        let (moved, _) = live
            .presence(&assumed.session, reach.clone())
            .expect("a document");
        assert_eq!(moved, first + 1);

        live.subscribe(&assumed.session, &a_loop("flight").id);
        let (again, _) = live.presence(&assumed.session, reach).expect("a document");
        assert_eq!(
            again, moved,
            "the version moved for a document that had not"
        );
    }

    /// **The grid overrules personalisation silently and always, and keeps it inert rather
    /// than dropping it** (ADR-0051). A loop that leaves reach leaves the document; a loop
    /// that comes back comes back where it was, so a temporary revocation does not destroy
    /// somebody's console arrangement.
    #[tokio::test]
    async fn a_subscription_outside_reach_is_kept_and_not_rendered() {
        let (_directory, store) = a_temporary_store().await;
        let (sign_in, user, role) = a_seat(&store, "flight", "Flight Director").await;
        let live = StateAuthority::empty();
        let assumed = live
            .assume(taking(&sign_in, &user, &role, Some(1)))
            .expect("the seat to be free");
        live.subscribe(&assumed.session, &a_loop("flight").id);

        assert!(
            monitoring(&live, &assumed.session, vec![a_loop("air-to-ground")]).is_empty(),
            "a loop out of reach was rendered"
        );

        assert_eq!(
            monitoring(&live, &assumed.session, vec![a_loop("flight")]),
            ["flight"],
            "reach came back and the subscription did not"
        );
    }

    /// The set is seeded from what Configuration remembers, handed in as a value: it is what
    /// makes a restart cost an assume rather than a rebuild (ADR-0050).
    #[tokio::test]
    async fn assuming_restores_the_set_the_pair_last_had() {
        let (_directory, store) = a_temporary_store().await;
        let (sign_in, user, role) = a_seat(&store, "flight", "Flight Director").await;
        let live = StateAuthority::empty();
        let assumed = live
            .assume(Assuming {
                subscribed_to: vec![a_loop("flight").id],
                ..taking(&sign_in, &user, &role, Some(1))
            })
            .expect("the seat to be free");

        assert_eq!(
            monitoring(
                &live,
                &assumed.session,
                vec![a_loop("air-to-ground"), a_loop("flight")]
            ),
            ["flight"]
        );
    }

    /// **A subscription is live state and ends with the session** (v1 §5). What outlives it
    /// is the memory of the set, which is not this module's.
    #[tokio::test]
    async fn a_subscription_ends_with_the_session_that_held_it() {
        let (_directory, store) = a_temporary_store().await;
        let (sign_in, user, role) = a_seat(&store, "flight", "Flight Director").await;
        let live = StateAuthority::empty();
        let assumed = live
            .assume(Assuming {
                subscribed_to: vec![a_loop("flight").id],
                ..taking(&sign_in, &user, &role, Some(1))
            })
            .expect("the seat to be free");
        live.ended_by_its_own_holder(&assumed.session)
            .expect("the session to end");

        assert!(!live.subscribe(&assumed.session, &a_loop("flight").id));
        assert!(!live.unsubscribe(&assumed.session, &a_loop("flight").id));
        let taken_again = live
            .assume(taking(&sign_in, &user, &role, Some(1)))
            .expect("the seat to be free");
        assert!(
            monitoring(&live, &taken_again.session, vec![a_loop("flight")]).is_empty(),
            "a new session inherited the last one's subscriptions"
        );
    }

    /// An act on a session that ended under it is refused rather than applied to somebody
    /// else's, and there is then nothing worth remembering.
    #[tokio::test]
    async fn subscribing_on_a_session_nobody_holds_changes_nothing() {
        let live = StateAuthority::empty();

        assert!(!live.subscribe(
            &SessionId::presented("nothing".to_owned()),
            &a_loop("flight").id
        ));
    }

    // ---- #40: the media path ladder ----------------------------------------------------

    /// Where the merged ladder stands, as the document would carry it.
    fn the_media_path(live: &StateAuthority, session: &SessionId) -> MediaPath {
        live.presence(session, Vec::new())
            .expect("a live session")
            .1
            .media_path
    }

    /// The rungs are ordered green to red, and the order is what makes the merge a `max`.
    /// A test rather than a comment because reordering the declaration would silently invert
    /// which reading wins when the two ends disagree.
    #[test]
    fn the_ladder_runs_from_connected_down_to_lost() {
        assert!(MediaPath::Connected < MediaPath::Impaired);
        assert!(MediaPath::Impaired < MediaPath::Lost);
        assert_eq!(MediaPath::default(), MediaPath::Lost);
    }

    /// A session that has just been minted has a transport being built and nobody connected
    /// to it. `lost` is the truth about that, and it is the truth the transmit bar has to be
    /// able to say.
    #[tokio::test]
    async fn a_session_starts_with_no_media_path_at_either_end() {
        let (_directory, store) = a_temporary_store().await;
        let (sign_in, user, role) = a_seat(&store, "flight", "Flight Director").await;
        let live = StateAuthority::empty();
        let assumed = live
            .assume(Assuming {
                sign_in,
                occupant: user,
                role,
                limit: None,
                subscribed_to: Vec::new(),
            })
            .expect("the seat to be free");

        assert_eq!(
            the_media_path(&live, &assumed.session),
            MediaPath::Lost,
            "a session was given a media path nobody had established"
        );
    }

    /// **Green needs both, red needs one** ([ADR-0042]). Every combination, because the two
    /// ends disagree routinely and which one wins is the whole of the rule.
    ///
    /// [ADR-0042]: ../../docs/adr/0042-the-media-path-has-its-own-ladder.md
    #[tokio::test]
    async fn the_two_ends_merge_pessimistically() {
        let (_directory, store) = a_temporary_store().await;
        let (sign_in, user, role) = a_seat(&store, "flight", "Flight Director").await;
        let live = StateAuthority::empty();
        let assumed = live
            .assume(Assuming {
                sign_in,
                occupant: user,
                role,
                limit: None,
                subscribed_to: Vec::new(),
            })
            .expect("the seat to be free");

        for (client, server, merged) in [
            (
                MediaPath::Connected,
                MediaPath::Connected,
                MediaPath::Connected,
            ),
            (
                MediaPath::Connected,
                MediaPath::Impaired,
                MediaPath::Impaired,
            ),
            (
                MediaPath::Impaired,
                MediaPath::Connected,
                MediaPath::Impaired,
            ),
            (MediaPath::Connected, MediaPath::Lost, MediaPath::Lost),
            (MediaPath::Lost, MediaPath::Connected, MediaPath::Lost),
            (MediaPath::Impaired, MediaPath::Lost, MediaPath::Lost),
            (MediaPath::Lost, MediaPath::Impaired, MediaPath::Lost),
        ] {
            live.the_client_says(&assumed.session, client);
            live.the_server_sees(&assumed.session, server);

            assert_eq!(
                the_media_path(&live, &assumed.session),
                merged,
                "the client said {client:?} and the server saw {server:?}"
            );
        }
    }

    /// A media path moving moves the document, because the document is the API and the
    /// transmit bar renders this.
    #[tokio::test]
    async fn the_version_moves_when_the_media_path_does() {
        let (_directory, store) = a_temporary_store().await;
        let (sign_in, user, role) = a_seat(&store, "flight", "Flight Director").await;
        let live = StateAuthority::empty();
        let assumed = live
            .assume(Assuming {
                sign_in,
                occupant: user,
                role,
                limit: None,
                subscribed_to: Vec::new(),
            })
            .expect("the seat to be free");
        let (first, _) = live
            .presence(&assumed.session, Vec::new())
            .expect("a document");

        live.the_client_says(&assumed.session, MediaPath::Connected);
        live.the_server_sees(&assumed.session, MediaPath::Connected);
        let (then, _) = live
            .presence(&assumed.session, Vec::new())
            .expect("a document");

        assert!(then > first, "the document moved and the version did not");

        // And it does not move for a reading that changes nothing.
        live.the_client_says(&assumed.session, MediaPath::Connected);
        let (again, _) = live
            .presence(&assumed.session, Vec::new())
            .expect("a document");
        assert_eq!(again, then);
    }

    /// The worker's death is the server's end everywhere at once, and it ends nothing. The
    /// operator is present, reading a working console that can say exactly what is wrong.
    #[tokio::test]
    async fn nothing_being_carried_takes_every_session_off_the_air_and_ends_none() {
        let (_directory, store) = a_temporary_store().await;
        let (one_sign_in, one_user, one_role) = a_seat(&store, "flight", "Flight Director").await;
        let (two_sign_in, two_user, two_role) = a_seat(&store, "capcom", "Capcom").await;
        let live = StateAuthority::empty();
        let one = live
            .assume(Assuming {
                sign_in: one_sign_in,
                occupant: one_user,
                role: one_role,
                limit: None,
                subscribed_to: Vec::new(),
            })
            .expect("the seat to be free");
        let two = live
            .assume(Assuming {
                sign_in: two_sign_in,
                occupant: two_user,
                role: two_role,
                limit: None,
                subscribed_to: Vec::new(),
            })
            .expect("the seat to be free");
        for session in [&one.session, &two.session] {
            live.the_client_says(session, MediaPath::Connected);
            live.the_server_sees(session, MediaPath::Connected);
        }

        live.nothing_is_carried();

        for session in [&one.session, &two.session] {
            assert_eq!(the_media_path(&live, session), MediaPath::Lost);
            assert!(
                live.the_role_of(session).is_some(),
                "a session was ended for a dead media path"
            );
        }
    }

    /// A reading for a session that ended under it is nothing, rather than a panic or a
    /// resurrection. It is what a client reporting into a seat it no longer holds finds.
    #[tokio::test]
    async fn a_reading_about_a_session_that_is_over_lands_nowhere() {
        let (_directory, store) = a_temporary_store().await;
        let (sign_in, user, role) = a_seat(&store, "flight", "Flight Director").await;
        let live = StateAuthority::empty();
        let assumed = live
            .assume(Assuming {
                sign_in,
                occupant: user,
                role,
                limit: None,
                subscribed_to: Vec::new(),
            })
            .expect("the seat to be free");
        live.ended_by_its_own_holder(&assumed.session);

        live.the_client_says(&assumed.session, MediaPath::Connected);
        live.the_server_sees(&assumed.session, MediaPath::Connected);

        assert!(live.presence(&assumed.session, Vec::new()).is_none());
    }

    // ---- Arming, keying and the fan-out (#41) ------------------------------------------

    /// A live session, made from its own user and its own role, so several can stand at once.
    async fn a_session(live: &StateAuthority, store: &Store, who: &str) -> SessionId {
        let (sign_in, user, role) = a_seat(store, who, &format!("{who}'s role")).await;

        live.assume(taking(&sign_in, &user, &role, Some(1)))
            .expect("the seat to be free")
            .session
    }

    /// The loops one session is armed on, as its own document has them.
    fn armed(live: &StateAuthority, session: &SessionId, within: Vec<InReach>) -> Vec<String> {
        live.presence(session, within)
            .expect("a document")
            .1
            .loops
            .into_iter()
            .filter(|standing| standing.armed)
            .map(|standing| standing.held_on.name)
            .collect()
    }

    /// The loops one session's document says are being spoken on.
    fn talking(live: &StateAuthority, session: &SessionId, within: Vec<InReach>) -> Vec<String> {
        live.presence(session, within)
            .expect("a document")
            .1
            .loops
            .into_iter()
            .filter(|standing| standing.talking)
            .map(|standing| standing.held_on.name)
            .collect()
    }

    /// Who hears this talker, and where, as the fan-out has it this instant.
    fn heard_by(live: &StateAuthority, talker: &SessionId) -> Vec<(String, String)> {
        live.the_routing_if_it_moved()
            .unwrap_or_default()
            .into_iter()
            .find(|who| &who.talker == talker)
            .map(|who| {
                who.listeners
                    .into_iter()
                    .map(|heard| {
                        (
                            heard.listener.as_str().to_owned(),
                            heard.on.as_str().to_owned(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// **Arming and subscription are independent in both directions** (ADR-0013). An arm puts
    /// a loop in nobody's ears — the arming operator's least of all — and a subscription
    /// makes no destination.
    #[tokio::test]
    async fn an_arm_never_enters_the_subscription_set_and_a_subscription_never_arms() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;
        let reach = vec![a_loop_to_emit_on("air-to-ground"), a_loop_to_emit_on("sim")];

        live.presence(&session, reach.clone());
        live.arm(&session, &LoopId::presented("air-to-ground".to_owned()));
        live.subscribe(&session, &LoopId::presented("sim".to_owned()));

        let (_, presence) = live.presence(&session, reach).expect("a document");
        let air_to_ground = &presence.loops[0];
        let sim = &presence.loops[1];

        assert!(air_to_ground.armed, "the armed loop is not armed");
        assert!(
            !air_to_ground.subscribed,
            "arming a loop put it in the operator's own ears"
        );
        assert!(sim.subscribed, "the monitored loop is not monitored");
        assert!(!sim.armed, "monitoring a loop armed it");
    }

    /// A second arm of the same loop is the same state, for the reason a second click is:
    /// nothing renders optimistically, so the control lags and a repeat must not undo the
    /// first.
    #[tokio::test]
    async fn arming_twice_before_the_control_catches_up_leaves_the_loop_armed() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;
        let reach = vec![a_loop_to_emit_on("air-to-ground")];
        live.presence(&session, reach.clone());

        live.arm(&session, &LoopId::presented("air-to-ground".to_owned()));
        live.arm(&session, &LoopId::presented("air-to-ground".to_owned()));
        live.disarm(&session, &LoopId::presented("air-to-ground".to_owned()));

        assert!(
            armed(&live, &session, reach).is_empty(),
            "one disarm did not undo two arms of the same loop"
        );
    }

    /// **An arm outside reach is dropped and a subscription outside reach is kept.** The
    /// asymmetry is deliberate: a preference restored with the cell leaves a console where it
    /// was, and a route restored the same way would put somebody back on the air with their
    /// hand on nothing.
    #[tokio::test]
    async fn a_revoked_cell_takes_the_arm_away_for_good_and_leaves_the_subscription_inert() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;
        let held_on = LoopId::presented("air-to-ground".to_owned());
        let emitting = vec![a_loop_to_emit_on("air-to-ground")];

        live.presence(&session, emitting.clone());
        live.arm(&session, &held_on);
        live.subscribe(&session, &held_on);

        // The cell goes to `none`, so the loop leaves the document altogether.
        live.presence(&session, Vec::new());

        let (_, back) = live.presence(&session, emitting).expect("a document");
        assert!(
            !back.loops[0].armed,
            "an arm came back on its own when the cell did"
        );
        assert!(
            back.loops[0].subscribed,
            "a subscription was destroyed by a revocation that was undone"
        );
    }

    /// A cell dropped to `monitor` is still in reach and still not somewhere this role may
    /// speak, so the arm goes with the rung rather than with the loop.
    #[tokio::test]
    async fn losing_emit_but_keeping_monitor_takes_the_arm_and_leaves_the_loop() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;
        live.presence(&session, vec![a_loop_to_emit_on("air-to-ground")]);
        live.arm(&session, &LoopId::presented("air-to-ground".to_owned()));

        let (_, presence) = live
            .presence(&session, vec![a_loop("air-to-ground")])
            .expect("a document");

        assert_eq!(presence.loops.len(), 1, "the loop left reach as well");
        assert!(
            !presence.loops[0].armed,
            "an arm outlived the rung under it"
        );
    }

    /// **The fan-out is every listener monitoring a loop the talker has armed**, and the loop
    /// crosses with each of them because the recording tap is per (talker, destination).
    #[tokio::test]
    async fn everybody_monitoring_an_armed_loop_hears_the_talker() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let talker = a_session(&live, &store, "flight").await;
        let listener = a_session(&live, &store, "capcom").await;
        let air_to_ground = LoopId::presented("air-to-ground".to_owned());

        live.presence(&talker, vec![a_loop_to_emit_on("air-to-ground")]);
        live.presence(&listener, vec![a_loop("air-to-ground")]);
        live.arm(&talker, &air_to_ground);
        live.subscribe(&listener, &air_to_ground);

        assert_eq!(
            heard_by(&live, &talker),
            vec![(listener.as_str().to_owned(), "air-to-ground".to_owned())]
        );
    }

    /// **The route stands whether or not anybody is keyed** (ADR-0008). Keying is the client
    /// muting its own microphone so that a press costs no round trip, and building the
    /// fan-out on the key signal would put the server back in the latency path and remove the
    /// residual that ADR accepts out loud.
    #[tokio::test]
    async fn the_fan_out_is_built_from_the_arm_and_not_from_the_key() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let talker = a_session(&live, &store, "flight").await;
        let listener = a_session(&live, &store, "capcom").await;
        let air_to_ground = LoopId::presented("air-to-ground".to_owned());
        live.presence(&talker, vec![a_loop_to_emit_on("air-to-ground")]);
        live.presence(&listener, vec![a_loop("air-to-ground")]);
        live.arm(&talker, &air_to_ground);
        live.subscribe(&listener, &air_to_ground);

        let unkeyed = heard_by(&live, &talker);
        live.the_client_keys(&talker);

        assert_eq!(
            unkeyed.len(),
            1,
            "an unkeyed talker had no route, so keying would cost a renegotiation"
        );
        assert_eq!(
            live.the_routing_if_it_moved(),
            None,
            "keying moved the fan-out"
        );
    }

    /// Nobody hears a loop the talker has not armed, and nobody hears a talker on a loop they
    /// are not monitoring. Two halves of the one rule, asserted together because a fan-out
    /// that got either wrong would look right from the other side.
    #[tokio::test]
    async fn an_unarmed_loop_and_an_unmonitored_one_carry_nothing() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let talker = a_session(&live, &store, "flight").await;
        let listener = a_session(&live, &store, "capcom").await;
        let reach = vec![a_loop_to_emit_on("air-to-ground"), a_loop_to_emit_on("sim")];
        live.presence(&talker, reach.clone());
        live.presence(&listener, reach);

        // Armed on one, and the listener is monitoring the other.
        live.arm(&talker, &LoopId::presented("air-to-ground".to_owned()));
        live.subscribe(&listener, &LoopId::presented("sim".to_owned()));

        assert!(heard_by(&live, &talker).is_empty());
    }

    /// A subscription outside reach is kept and inert (ADR-0051), and inert has to mean
    /// inaudible: a loop that is out of the document must not be in somebody's ears.
    #[tokio::test]
    async fn a_subscription_the_role_may_no_longer_monitor_hears_nothing() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let talker = a_session(&live, &store, "flight").await;
        let listener = a_session(&live, &store, "capcom").await;
        let air_to_ground = LoopId::presented("air-to-ground".to_owned());
        live.presence(&talker, vec![a_loop_to_emit_on("air-to-ground")]);
        live.presence(&listener, vec![a_loop("air-to-ground")]);
        live.arm(&talker, &air_to_ground);
        live.subscribe(&listener, &air_to_ground);
        assert_eq!(heard_by(&live, &talker).len(), 1);

        // The listener's cell goes to `none`. The subscription stands and stops being heard.
        live.presence(&listener, Vec::new());

        assert!(
            heard_by(&live, &talker).is_empty(),
            "a loop out of reach was still in somebody's ears"
        );
    }

    /// Hearing yourself back over the network is a fault in an intercom, not a feature.
    #[tokio::test]
    async fn a_talker_is_not_in_their_own_audience() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let talker = a_session(&live, &store, "flight").await;
        let air_to_ground = LoopId::presented("air-to-ground".to_owned());
        live.presence(&talker, vec![a_loop_to_emit_on("air-to-ground")]);
        live.arm(&talker, &air_to_ground);
        live.subscribe(&talker, &air_to_ground);

        assert!(heard_by(&live, &talker).is_empty());
    }

    /// **One listener, two destinations, and both are handed down.** The downlink is one
    /// stream per audible talker (ADR-0007) — collapsing the pair is the media plane's to
    /// do, and it can only do it if it is told what it is collapsing, because the recording
    /// tap is per (talker, destination loop).
    #[tokio::test]
    async fn a_listener_monitoring_two_of_a_talkers_loops_is_named_on_both() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let talker = a_session(&live, &store, "flight").await;
        let listener = a_session(&live, &store, "capcom").await;
        let reach = vec![a_loop_to_emit_on("air-to-ground"), a_loop_to_emit_on("sim")];
        live.presence(&talker, reach.clone());
        live.presence(&listener, reach);

        for held_on in ["air-to-ground", "sim"] {
            live.arm(&talker, &LoopId::presented(held_on.to_owned()));
            live.subscribe(&listener, &LoopId::presented(held_on.to_owned()));
        }

        assert_eq!(
            heard_by(&live, &talker),
            vec![
                (listener.as_str().to_owned(), "air-to-ground".to_owned()),
                (listener.as_str().to_owned(), "sim".to_owned())
            ]
        );
    }

    /// **It is taken rather than read**, and it moves when it moves. A sink is handed the
    /// whole audience each time, so an unchanged one is an instruction to rebuild what is
    /// already there — and two sockets asking on the same tick must not both carry it down.
    #[tokio::test]
    async fn the_routing_is_handed_down_once_and_again_only_when_it_has_moved() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let talker = a_session(&live, &store, "flight").await;
        live.presence(&talker, vec![a_loop_to_emit_on("air-to-ground")]);

        assert!(
            live.the_routing_if_it_moved().is_some(),
            "a deployment with a session in it had no routing at all"
        );
        assert_eq!(live.the_routing_if_it_moved(), None);

        live.arm(&talker, &LoopId::presented("air-to-ground".to_owned()));
        // The arm reaches nobody, so the answer is the same one and is not handed down again.
        assert_eq!(live.the_routing_if_it_moved(), None);
    }

    /// **The indicator marks the loop, never the talker**, and it is one flag rather than a
    /// list: identical for one talker and for five (ADR-0033).
    #[tokio::test]
    async fn the_talking_indicator_is_the_same_for_one_talker_and_for_two() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let one = a_session(&live, &store, "flight").await;
        let other = a_session(&live, &store, "capcom").await;
        let watching = a_session(&live, &store, "gnc").await;
        let air_to_ground = LoopId::presented("air-to-ground".to_owned());
        let emitting = vec![a_loop_to_emit_on("air-to-ground")];
        for session in [&one, &other] {
            live.presence(session, emitting.clone());
            live.arm(session, &air_to_ground);
        }

        assert!(
            talking(&live, &watching, vec![a_loop("air-to-ground")]).is_empty(),
            "an armed loop nobody is keyed on reads as being spoken on"
        );

        live.the_client_keys(&one);
        let one_talker = talking(&live, &watching, vec![a_loop("air-to-ground")]);
        live.the_client_keys(&other);
        let two_talkers = talking(&live, &watching, vec![a_loop("air-to-ground")]);

        assert_eq!(one_talker, vec!["air-to-ground".to_owned()]);
        assert_eq!(two_talkers, one_talker, "the indicator counted its talkers");

        live.the_client_unkeys(&one);
        live.the_client_unkeys(&other);
        assert!(talking(&live, &watching, vec![a_loop("air-to-ground")]).is_empty());
    }

    /// **Every armed loop shows whether somebody is transmitting on it** (v1 §4), and that is
    /// the whole compensation for emitting blind: the loop is armed, unmonitored, and the
    /// operator can still see they are about to talk over somebody.
    #[tokio::test]
    async fn a_blind_armed_loop_still_says_somebody_is_talking_on_it() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let blind = a_session(&live, &store, "flight").await;
        let talker = a_session(&live, &store, "capcom").await;
        let air_to_ground = LoopId::presented("air-to-ground".to_owned());
        let emitting = vec![a_loop_to_emit_on("air-to-ground")];
        live.presence(&blind, emitting.clone());
        live.presence(&talker, emitting.clone());
        live.arm(&blind, &air_to_ground);
        live.arm(&talker, &air_to_ground);
        live.the_client_keys(&talker);

        let (_, presence) = live.presence(&blind, emitting).expect("a document");

        assert!(presence.loops[0].armed);
        assert!(!presence.loops[0].subscribed, "the arm subscribed somebody");
        assert!(presence.loops[0].talking, "a blind arm was left blind");
    }

    /// **The lamp is the server's answer** (ADR-0008). It is a field of the document, so the
    /// console has nothing else to light it from and no way to pre-light it.
    #[tokio::test]
    async fn the_transmitting_lamp_is_a_field_of_the_document() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;

        assert!(!live.is_keyed(&session));
        assert!(
            !live
                .presence(&session, Vec::new())
                .expect("a document")
                .1
                .keyed
        );

        live.the_client_keys(&session);

        assert!(live.is_keyed(&session));
        assert!(
            live.presence(&session, Vec::new())
                .expect("a document")
                .1
                .keyed
        );
    }

    /// The key moves the document, so the lamp arrives on the acknowledgement rather than on
    /// the next thing that happens to change.
    #[tokio::test]
    async fn keying_moves_the_version() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;
        let (before, _) = live.presence(&session, Vec::new()).expect("a document");

        live.the_client_keys(&session);
        let (after, _) = live.presence(&session, Vec::new()).expect("a document");

        assert_eq!(after, before + 1);
    }

    /// An act on a session that ended under it changes nothing, and says so.
    #[tokio::test]
    async fn arming_and_keying_a_session_that_has_ended_do_nothing() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;
        live.ended_by_its_own_holder(&session);

        assert!(!live.arm(&session, &LoopId::presented("air-to-ground".to_owned())));
        assert!(!live.disarm(&session, &LoopId::presented("air-to-ground".to_owned())));
        assert!(!live.the_client_keys(&session));
        assert!(!live.the_client_unkeys(&session));
        assert!(!live.is_keyed(&session));
    }

    /// One loop in reach, named the way a grid row hands it over.
    fn a_loop(name: &str) -> InReach {
        InReach {
            id: LoopId::presented(name.to_owned()),
            name: name.to_owned(),
            permission: Permission::Monitor,
        }
    }

    /// The same loop, on a row that may speak on it.
    fn a_loop_to_emit_on(name: &str) -> InReach {
        InReach {
            permission: Permission::Emit,
            ..a_loop(name)
        }
    }
}
