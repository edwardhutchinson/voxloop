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
use std::time::{Duration, Instant, SystemTime};

use crate::configuration::{Ladder, LoopId, Permission, RoleId, SignInToken, UserId, Volume};
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

/// A session's standing with the signalling channel ([ADR-0018]).
///
/// The first of the three axes any console state has to be read against, and it describes
/// the **state channel and never the audio path** — that is [`MediaPath`], and the two fail
/// independently in both directions.
///
/// **A session with no signalling channel has no emission path.** Every talking indicator
/// anyone sees is a server broadcast ([ADR-0008]), so an operator keying with no channel
/// transmits into a system where nobody's console shows them, no loop attributes it and no
/// authority holder can cut it. The audio arrives; the accountability does not.
///
/// `Unconfirmed` is the band that makes withdrawing emission safe rather than fragile. A
/// single threshold would mean a VPN reroute mutes the Flight Director mid-sentence, trading
/// a state-honesty problem for a worse availability one. Its honest reading is *we cannot
/// confirm your transmission right now*, which is a materially different statement from *we
/// know you are disconnected* — so emission still stands on it.
///
/// **The order of these lines is the ladder** and it is derived from the declaration order,
/// so moving one of them changes which rung a session is read at.
///
/// [ADR-0008]: ../../docs/adr/0008-emission-is-armed-by-the-server-and-keyed-by-the-client.md
/// [ADR-0018]: ../../docs/adr/0018-no-signalling-channel-means-no-emission-path.md
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Connection {
    /// Heartbeats current: everything normal.
    #[default]
    Confirmed,
    /// Heartbeats missed. The console's displayed state is frozen and marked stale with a
    /// running age, and **push-to-talk stays live**.
    Unconfirmed,
    /// Past the threshold. The client disables push-to-talk and the server closes the
    /// fan-out — the client-side half alone would be a courtesy in exactly the situation
    /// where the client may be wedged.
    Disconnected,
}

impl Connection {
    /// The word this rung goes by, which is the word `CONTEXT.md` and the spec use.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::Unconfirmed => "unconfirmed",
            Self::Disconnected => "disconnected",
        }
    }

    /// Whether emission stands at this rung.
    ///
    /// The whole of ADR-0018's server-side rule, in one place so that the fan-out and the
    /// talking indicator cannot come to disagree about it.
    fn carries_emission(self) -> bool {
        self != Self::Disconnected
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

/// How often each loop's beacon sounds ([ADR-0017]).
///
/// **The media plane sounds it and this module measures it**, so the one number both sides
/// read lives beside the measurement: the window a beacon may go unheard in is a multiple of
/// it, and two copies of the interval would be two numbers that could drift apart until every
/// loop read as lost.
///
/// Every five seconds, which is the arithmetic v1 §6 gives: around 200 sessions subscribed to
/// roughly six loops each is 1,200 beacon carriages, and one packet each every five seconds is
/// the 240 packets per second the spec names against the tens of thousands live speech sends.
///
/// [ADR-0017]: ../../docs/adr/0017-loop-health-is-measured-not-asserted.md
pub(crate) const THE_BEACON_SOUNDS_EVERY: Duration = Duration::from_secs(5);

/// How long a beacon may go uncounted before the loop is read as not received.
///
/// **Three intervals, so two packets in a row may go missing** before anything is said. The
/// beacon rides UDP like the speech it stands in for, and a loop that flapped to *not
/// receiving* on every lost packet would teach an operator to ignore the one reading this
/// product can least afford to have ignored.
///
/// It is longer than the signalling ladder's `disconnected` threshold on purpose. The counts
/// ride the signalling channel, so a channel that has gone stops them as a side effect — and
/// by the time this window could run out on that, connection state has already said what is
/// wrong, which is the suppression v1 §6 asks for.
const A_BEACON_IS_LOST_AFTER: Duration = Duration::from_secs(15);

/// Whether a session is actually receiving a loop: the third axis ([ADR-0017]).
///
/// **It is measured, never asserted.** The server's belief that a carriage exists and is
/// unpaused is nearly always green, because it reports a belief about a media path rather
/// than the path; this is the arrival of the loop's beacon, counted at the far end. DTX means
/// a quiet loop and an unreachable loop sound identical, so **they must never look
/// identical**, and this is what tells them apart.
///
/// **It is per (session, loop)**, so two subscribers may correctly disagree.
///
/// **What it proves is that the loop reaches this session, and no more.** The downlink is per
/// talker (ADR-0007), so the beacon's carriage is not the one carrying anybody's voice: loss
/// soundly proves deafness, and arrival does not prove any given talker would be heard. That
/// gap is recorded rather than closed (v1 §16), and nothing here may be read as closing it.
///
/// [ADR-0017]: ../../docs/adr/0017-loop-health-is-measured-not-asserted.md
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LoopHealth {
    /// The beacon has been counted within the window: the loop reaches this session.
    Receiving,
    /// Nothing has been counted since the loop was taken up, or since the channel came back,
    /// and the window has not run out on it yet. **It is a measurement not yet taken rather
    /// than a failure**, and it becomes one if nothing arrives.
    Checking,
    /// Nothing has been counted for the whole window. The session is deaf to this loop, and
    /// a quiet loop would sound no different.
    NotReceiving,
}

impl LoopHealth {
    /// The word the presence document carries.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Receiving => "receiving",
            Self::Checking => "checking",
            Self::NotReceiving => "not-receiving",
        }
    }
}

/// Why one occupant is not hearing a loop, as staffing state reads it (v1 §1, §8).
///
/// **The order of these lines is the order a reason is chosen in**, furthest upstream first:
/// each is still true if everything below it were fixed. That is what makes connection state
/// win over beacon loss — the suppression v1 §6 asks for, generalised — and it is why there is
/// exactly one reason per occupant rather than a list.
///
/// *Off console* sits between the first two. Staffing state — counted across every occupant
/// of every staffing role — is built from this fact about each occupant and from no other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NotHearing {
    /// The signalling channel is gone. Nothing this session reports is arriving, beacon
    /// counts included, so this is the one reason that stands in for all the others.
    Unreachable,
    /// **The occupant has said they are not in the chair** ([ADR-0016]). It is the one
    /// asserted reason among observed ones, and it sits second because the conditions nest:
    /// it is still true if everything below it were fixed, and it is not true of a session
    /// that cannot be reached at all — that one explains the silence on its own.
    ///
    /// [ADR-0016]: ../../docs/adr/0016-displayed-state-is-observed-or-asserted.md
    OffConsole,
    /// The loop is not on this console.
    NotSubscribed,
    /// **The loop's beacon is not arriving** ([ADR-0017]). It is what upgrades `staffed` from
    /// *says they are listening* to *demonstrably receiving*, and it fails safe for a wedged
    /// client, which reports nothing and so is counted as receiving nothing.
    ///
    /// [ADR-0017]: ../../docs/adr/0017-loop-health-is-measured-not-asserted.md
    NotReceiving,
    /// The operator has silenced it in their own ears.
    Muted,
}

impl NotHearing {
    /// The five, **in the order a reason is chosen in** and so in the order they are
    /// counted and read out: furthest upstream first.
    ///
    /// It is written out rather than derived, because the order is the model here and a
    /// list that happened to agree with the enum today is one that silently stops agreeing
    /// when somebody adds a sixth reason in the middle.
    const FURTHEST_UPSTREAM_FIRST: [Self; 5] = [
        Self::Unreachable,
        Self::OffConsole,
        Self::NotSubscribed,
        Self::NotReceiving,
        Self::Muted,
    ];

    /// The word the presence document and the lobby carry.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Unreachable => "unreachable",
            Self::OffConsole => "off-console",
            Self::NotSubscribed => "not-subscribed",
            Self::NotReceiving => "not-receiving",
            Self::Muted => "muted",
        }
    }
}

/// One loop and the roles marked as staffing it, as Configuration answered it.
///
/// **It is handed in as a value** ([ADR-0039]): which roles staff which loops is durable
/// configuration and this module reads no store, so the two sides meet the way blast radius
/// makes them meet. A loop with no staffing roles is simply not in the list handed over,
/// which is how [ADR-0056]'s absence arrives here — there is no fourth value to represent it
/// with and nothing to configure.
///
/// [ADR-0039]: ../../docs/adr/0039-live-state-is-in-process-behind-one-state-authority.md
/// [ADR-0056]: ../../docs/adr/0056-a-loop-with-no-staffing-roles-has-no-staffing-state.md
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StaffedBy {
    pub(crate) held_on: LoopId,
    pub(crate) roles: Vec<RoleId>,
}

/// Whether a human is behind a loop (v1 §1).
///
/// Three values and no partial one: it is computed across **every occupant of every**
/// staffing role for the loop, so one occupant going quiet moves nothing while another is
/// still hearing it. The fourth case — a loop with no staffing roles — is the absence of
/// this type rather than a value of it ([ADR-0056]).
///
/// [ADR-0056]: ../../docs/adr/0056-a-loop-with-no-staffing-roles-has-no-staffing-state.md
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Staffing {
    /// An occupant of a staffing role is **demonstrably hearing** it ([ADR-0005]): their
    /// channel is up, they have not stepped away, the loop is on their console unmuted, and
    /// its beacon is arriving.
    ///
    /// [ADR-0005]: ../../docs/adr/0005-occupancy-means-listening-not-signed-in.md
    Staffed,
    /// Such occupants exist and none of them is hearing it, **with the reason** — counted,
    /// because they can be away for different reasons at once and no ordering across people
    /// is defensible ([ADR-0065]).
    ///
    /// In [`NotHearing::FURTHEST_UPSTREAM_FIRST`] order and never empty: the variant is
    /// only ever reached by counting at least one occupant who is not hearing it.
    ///
    /// [ADR-0065]: ../../docs/adr/0065-the-staffing-flag-reports-it-never-subscribes.md
    Away(Vec<(NotHearing, usize)>),
    /// Nobody occupies a staffing role. **A service principal is not an occupant**: its
    /// binding gives reach and never occupancy ([ADR-0027]), and it holds no session, so
    /// nothing here can count it.
    ///
    /// [ADR-0027]: ../../docs/adr/0027-a-service-principal-acts-through-a-role.md
    Vacant,
}

impl Staffing {
    /// The word the board carries and the sentence in the ledger is built from.
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Staffed => "staffed",
            Self::Away(_) => "away",
            Self::Vacant => "vacant",
        }
    }
}

/// One loop's beacon, as this session's client has counted it.
///
/// It is kept per subscription, beside the set rather than inside it, because it is a
/// measurement and the set is a choice: taking a loop up starts one, and dropping the loop
/// drops it.
struct Counting {
    held_on: LoopId,
    /// The client's running count of this beacon's packets, as it last said it.
    counted: u64,
    /// When the count last moved, where it has moved since the measurement began.
    arrived: Option<Instant>,
    /// When the measurement began: the loop being taken up, or the channel coming back.
    since: Instant,
}

impl Counting {
    fn starting(held_on: LoopId, now: Instant) -> Self {
        Self {
            held_on,
            counted: 0,
            arrived: None,
            since: now,
        }
    }

    /// Take a count. **A count that moved is an arrival and a count said again is not**: the
    /// client says what it has, and the same number twice is it saying that nothing came.
    ///
    /// A count that went down is a carriage the client built afresh and started counting from
    /// nothing, and anything above nothing on it is a packet that arrived.
    fn counted(&mut self, packets: u64, now: Instant) {
        if packets != self.counted && packets > 0 {
            self.arrived = Some(now);
        }
        self.counted = packets;
    }

    /// Start measuring again, as of now, forgetting when the beacon last arrived.
    fn again(&mut self, now: Instant) {
        self.arrived = None;
        self.since = now;
    }

    fn health(&self, now: Instant) -> LoopHealth {
        match self.arrived {
            Some(at) if now.saturating_duration_since(at) < A_BEACON_IS_LOST_AFTER => {
                LoopHealth::Receiving
            }
            None if now.saturating_duration_since(self.since) < A_BEACON_IS_LOST_AFTER => {
                LoopHealth::Checking
            }
            _ => LoopHealth::NotReceiving,
        }
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
    /// When this session was last heard from on the signalling channel, which is what its
    /// connection state is measured from ([ADR-0018]).
    ///
    /// It starts at the assume that minted the session rather than at nothing, because a
    /// seat just taken has demonstrably been heard from — the act that created it arrived on
    /// the socket that is about to carry its heartbeats.
    ///
    /// [ADR-0018]: ../../docs/adr/0018-no-signalling-channel-means-no-emission-path.md
    heard_from: Instant,
    /// Whether the channel is **known** to have gone, rather than merely unheard from.
    ///
    /// A socket that closed is a fact rather than a silence, so it does not wait out the
    /// ladder: a tab closed, a browser quit or a network that reset is `disconnected` at
    /// once, and the fan-out closes with it. The rungs are for the case nobody reported —
    /// the wedged client, the flapping VPN — where all the server has is a gap.
    ///
    /// **It does not end the session.** Occupancy survives the loss of the channel and is
    /// held for the reconnection window (#50); this is the standing of the channel, not of
    /// the seat.
    the_channel_is_gone: bool,
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
    /// Each subscribed loop's beacon, as this session's client has counted it ([ADR-0017]).
    ///
    /// **One per subscription, whatever else is true of it**: a muted loop is still consumed,
    /// because a mute is not an unsubscribe, and a loop outside reach keeps its entry for the
    /// reason the subscription does ([ADR-0051]) — it is inert, and its beacon is not carried.
    ///
    /// [ADR-0017]: ../../docs/adr/0017-loop-health-is-measured-not-asserted.md
    /// [ADR-0051]: ../../docs/adr/0051-personalisation-is-scoped-to-the-smallest-thing-it-is-about.md
    beacons: Vec<Counting>,
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
    /// Whether the arm set was last moved by something other than this session.
    ///
    /// **Only a change the session did not ask for is marked** ([ADR-0058]). A deliberate arm
    /// and a preset are the routine mid-key changes — a preset is one by design, and the most
    /// routine thing in the system — so marking those would fire the signal constantly and
    /// train the operator straight past it. What is left is the change no hand on this desk
    /// made: an administrator pulling an `emit` cell while somebody is mid-sentence, and
    /// anything later that moves an arm set from outside the session.
    ///
    /// **It costs one flag on the update**, which is the whole of the mechanism: the server
    /// applies both kinds of change and is the one thing that knows which is which.
    ///
    /// It stands until the operator does something deliberate, which is the rule the one
    /// asserted state is already cleared by ([ADR-0016]): an act on this console is the
    /// evidence that the person at it has read what is on it. There is deliberately no
    /// dismissal of its own — a mark that had to be clicked away would be a second control on
    /// the strip an operator reads in the second before keying.
    ///
    /// [ADR-0016]: ../../docs/adr/0016-displayed-state-is-observed-or-asserted.md
    /// [ADR-0058]: ../../docs/adr/0058-the-transmit-bar-is-live-while-keyed.md
    arms_moved_elsewhere: bool,
    /// The loops this session has silenced in its own ears.
    ///
    /// **A mute is not an unsubscribe** (v1 §5), so it is a list beside the subscription set
    /// rather than a removal from it: the subscription stands, and with it everything that
    /// rides on one — the talking indicator, loop health, the priority mark. What a mute
    /// takes away is the audio, which is why [`Session::hears`] reads it.
    ///
    /// **It presupposes a subscription** ([ADR-0049]), so nothing is here that is not in the
    /// set above, and dropping a loop drops its mute with it.
    ///
    /// **It is never remembered** ([ADR-0050]). A forgotten mute silences a loop the moment
    /// its owner assumes the role again, and drops every loop they staff to `away` before
    /// they have looked at anything — so a seat just taken starts with none, whatever the
    /// last session had. Nor does one expire: nothing here has a clock, because an
    /// unexpected un-mute mid-incident is its own hazard.
    ///
    /// [ADR-0049]: ../../docs/adr/0049-the-role-is-the-profile.md
    /// [ADR-0050]: ../../docs/adr/0050-personalisation-persists-what-is-safe-to-be-stale.md
    mutes: Vec<LoopId>,
    /// How loud each loop plays in this operator's ears, for the loops they have set.
    ///
    /// **A loop missing from here is at unity**, which is where every loop starts (v1 §10).
    /// It is seeded from what Configuration remembers at assume, the way the subscription
    /// set is, and like it, it is not narrowed to reach: a loop that leaves reach and comes
    /// back comes back at the level it left at ([ADR-0051]).
    ///
    /// **It is not a route**, and nothing in the fan-out reads it. Loudest-wins is settled by
    /// the client over every loop a talker reaches it on ([ADR-0007]), and a loop turned all
    /// the way down is still a loop the operator is monitoring. Silencing one outright is
    /// what a mute is for.
    ///
    /// [ADR-0007]: ../../docs/adr/0007-the-client-emits-one-stream.md
    /// [ADR-0051]: ../../docs/adr/0051-personalisation-is-scoped-to-the-smallest-thing-it-is-about.md
    volumes: Vec<(LoopId, Volume)>,
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
    /// The priority press this session's client says is being held, where one is.
    ///
    /// **Priority is a second level beside the key and not a kind of key** ([ADR-0046]). The
    /// client ORs its levels and says the answer as `keyed`; this is the priority level on its
    /// own, and `is-priority` is read off it only while there is a transmission for it to be an
    /// attribute of. It is held as the press rather than as a flag because **every press is
    /// audited** with the arm set it was keyed over and how long it lasted (v1 §12), and both
    /// are facts about the moment the key went down.
    ///
    /// It carries no loop. **Priority applies to the whole arm set** ([ADR-0045]): one stream
    /// fanned out at the server cannot be priority on one armed loop and ordinary on another.
    ///
    /// [ADR-0045]: ../../docs/adr/0045-priority-defeats-attenuation-and-nothing-else.md
    /// [ADR-0046]: ../../docs/adr/0046-priority-is-keyed-not-held.md
    pressing: Option<Pressing>,
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
    /// Whether this operator has said they are not in the chair.
    ///
    /// **The one asserted state in VoxLoop** ([ADR-0016]), and the only field here that is a
    /// claim rather than something the server saw. Nothing derives it: idle-based auto-away
    /// is rejected outright, because an operator watching telemetry is idle at the keyboard
    /// and very much on console, so a clock that flipped this would be a fabrication in the
    /// opposite direction from the one it was meant to fix.
    ///
    /// **It is never remembered** ([ADR-0050]), so a seat just taken starts on console
    /// whatever the last session in it claimed: a day-old assertion is not a fact about
    /// anything.
    ///
    /// [ADR-0016]: ../../docs/adr/0016-displayed-state-is-observed-or-asserted.md
    /// [ADR-0050]: ../../docs/adr/0050-personalisation-persists-what-is-safe-to-be-stale.md
    off_console: bool,
    /// When this session last did something deliberate — the observed fact the assertion
    /// above is shown against.
    ///
    /// **An assertion is only as true as the moment it was made**, so it is never shown
    /// without this ([ADR-0016]). It is free, because every deliberate act already arrives
    /// here as a signalling message and Transport is the one thing that can tell a person's
    /// act from the machine's: a heartbeat, a media path report and a beacon count are a tab
    /// noticing things about itself and move nothing here.
    ///
    /// It starts at the assume that minted the session, which is the most deliberate thing
    /// anybody has done in it.
    ///
    /// [ADR-0016]: ../../docs/adr/0016-displayed-state-is-observed-or-asserted.md
    last_active: Instant,
}

impl Session {
    /// Where this session stands with the signalling channel, as of `now`.
    ///
    /// **It is derived rather than stored**, which is what keeps it from going stale: a rung
    /// held as a field would have to be moved by somebody remembering to move it, and the
    /// one case that matters is the case where nothing is arriving to remind them. The
    /// answer is a gap and two thresholds, computed wherever it is read.
    fn connection(&self, by: Ladder, now: Instant) -> Connection {
        if self.the_channel_is_gone {
            return Connection::Disconnected;
        }

        match now.saturating_duration_since(self.heard_from) {
            unheard if unheard >= by.disconnected() => Connection::Disconnected,
            unheard if unheard >= by.unconfirmed() => Connection::Unconfirmed,
            _ => Connection::Confirmed,
        }
    }

    /// Whether this session is on the air, which is its own claim **and** a channel to make
    /// it on.
    ///
    /// The key is the client's claim ([ADR-0008]) and it is the last one it managed to send.
    /// A session past the disconnect threshold is not transmitting whatever it last said,
    /// because the server has closed its fan-out and there is nowhere for the voice to go —
    /// so the talking indicator and the fan-out are read off one answer rather than two that
    /// agree until one of them is edited.
    ///
    /// [ADR-0008]: ../../docs/adr/0008-emission-is-armed-by-the-server-and-keyed-by-the-client.md
    fn is_transmitting(&self, by: Ladder, now: Instant) -> bool {
        self.keyed && self.connection(by, now).carries_emission()
    }

    /// Whether this session is on the air **at priority**: `is-priority = priority-level`, of a
    /// transmission that is landing ([ADR-0046]).
    ///
    /// It is read off [`Session::is_transmitting`] rather than beside it, so that whatever takes
    /// a transmission off the air takes its priority with it. A talker whose fan-out is closed
    /// is marked nowhere — which is what makes **Cut beat priority** ([ADR-0045]) by
    /// construction rather than by an ordering somebody has to remember.
    ///
    /// [ADR-0045]: ../../docs/adr/0045-priority-defeats-attenuation-and-nothing-else.md
    /// [ADR-0046]: ../../docs/adr/0046-priority-is-keyed-not-held.md
    fn is_at_priority(&self, by: Ladder, now: Instant) -> bool {
        self.pressing.is_some() && self.is_transmitting(by, now)
    }

    /// Let go of the priority press this session holds, and hand it back to be audited.
    fn let_go(&mut self) -> Option<Pressed> {
        let pressing = self.pressing.take()?;

        Some(Pressed {
            occupant: self.occupant.clone(),
            role: self.role.clone(),
            armed_on: pressing.armed_on,
            at: pressing.at,
            lasted: pressing.began.elapsed(),
        })
    }

    /// What the two ends amount to. Green needs both, red needs one.
    fn media_path(&self) -> MediaPath {
        self.said_by_the_client
            .pessimistically_with(self.seen_by_the_server)
    }

    /// The assertion this operator has made about themselves, where they have made one.
    ///
    /// **The claim and the age of its evidence are one answer**, so there is no way to render
    /// the first without the second ([ADR-0016]): a console handed the assertion on its own
    /// would be free to draw it as though the server had seen it, which is the one thing the
    /// rule forbids. Nothing at all where nobody has claimed anything — *on console* is the
    /// absence of an assertion rather than a second one.
    ///
    /// [ADR-0016]: ../../docs/adr/0016-displayed-state-is-observed-or-asserted.md
    fn asserted(&self, now: Instant) -> Option<Asserted> {
        self.off_console.then(|| Asserted {
            // **Whole seconds, because the document's version moves when this does.** The age
            // is genuinely part of the state — a claim and how old its evidence is are one
            // answer — so a full-precision duration here would make every 200 ms tick a new
            // version carrying byte-identical JSON, and *is this the same state* is the one
            // question versioning answers ([ADR-0019]). Seconds is also the resolution the
            // wire carries and the console renders, so nothing is lost by rounding here
            // rather than at the edge.
            //
            // [ADR-0019]: ../../docs/adr/0019-presence-is-one-versioned-document-scoped-to-reach.md
            last_active: Duration::from_secs(
                now.saturating_duration_since(self.last_active).as_secs(),
            ),
        })
    }

    /// Whether this session may hear that loop: it is in reach, it is monitored, and it is
    /// not muted.
    ///
    /// The first two are needed and neither implies the other. The rung says what this role
    /// may ever hear and the subscription says what it is hearing now (v1 §5), and a
    /// subscription outside reach is kept precisely so that it can be inert ([ADR-0051]).
    ///
    /// **The third is the operator's own**, and it is read here rather than at the client
    /// because a mute silences the loop in their ears and nowhere else: the fan-out stops
    /// carrying a talker to them on it, and everybody else on the loop is untouched. It is
    /// also what makes a mute sovereign over priority ([ADR-0045]) — a priority transmission
    /// raises the gain on a carriage, and there is no carriage.
    ///
    /// [ADR-0045]: ../../docs/adr/0045-priority-defeats-attenuation-and-nothing-else.md
    /// [ADR-0051]: ../../docs/adr/0051-personalisation-is-scoped-to-the-smallest-thing-it-is-about.md
    fn hears(&self, held_on: &LoopId) -> bool {
        self.subscriptions.contains(held_on)
            && !self.mutes.contains(held_on)
            && self.reach.iter().any(|within| &within.id == held_on)
    }

    /// Whether this session is monitoring that loop within its reach, muted or not.
    ///
    /// It is [`Session::hears`] without the mute, and it is what the beacon is carried on:
    /// **a mute is not an unsubscribe**, so a muted loop's beacon keeps arriving and so does
    /// its health (v1 §5).
    fn monitors(&self, held_on: &LoopId) -> bool {
        self.subscriptions.contains(held_on)
            && self.reach.iter().any(|within| &within.id == held_on)
    }

    /// This session's health on that loop, where there is one to show.
    ///
    /// Nothing for a loop it is not monitoring, because there is no beacon to measure. And
    /// **nothing while the signalling channel is not confirmed**: the counts ride that channel,
    /// so a channel in trouble stops them as a side effect, and a loop shown as not received
    /// beside a console marked stale would be one failure arriving as two competing reasons
    /// (v1 §6). Connection state answers for the loop until the channel is back.
    fn health_of(&self, held_on: &LoopId, by: Ladder, now: Instant) -> Option<LoopHealth> {
        if !self.monitors(held_on) || self.connection(by, now) != Connection::Confirmed {
            return None;
        }

        self.beacons
            .iter()
            .find(|counting| &counting.held_on == held_on)
            .map(|counting| counting.health(now))
    }

    /// Why this occupant is not hearing that loop, furthest upstream first, or nothing where
    /// they are.
    ///
    /// **Checking is not a reason**: it is a measurement not yet taken, and a loop taken up a
    /// second ago that dropped to `away` until its first beacon landed would put a reason on
    /// every loop somebody touched. It becomes one when the window runs out on it.
    #[cfg_attr(not(test), allow(dead_code))]
    fn not_hearing(&self, held_on: &LoopId, by: Ladder, now: Instant) -> Option<NotHearing> {
        if self.connection(by, now) == Connection::Disconnected {
            return Some(NotHearing::Unreachable);
        }
        if self.off_console {
            return Some(NotHearing::OffConsole);
        }
        if !self.monitors(held_on) {
            return Some(NotHearing::NotSubscribed);
        }
        if self.health_of(held_on, by, now) == Some(LoopHealth::NotReceiving) {
            return Some(NotHearing::NotReceiving);
        }
        if self.mutes.contains(held_on) {
            return Some(NotHearing::Muted);
        }

        None
    }

    /// Which bucket of a talker's audience this session is in, over that talker's whole arm
    /// set (v1 §6).
    ///
    /// **Hearing any one of the armed loops is hearing**, so the arm set is read until the
    /// first loop this session is hearing and the answer is settled there. Where none of them
    /// is being heard, what separates the warning from the bucket nobody is shown is whether
    /// this session took **any** of the loops up: somebody monitoring one of them and not
    /// hearing it believes they are covering it, and that is the whole of what
    /// `present, not hearing` means ([ADR-0034]).
    ///
    /// It is deliberately not the furthest-upstream reason staffing state reads
    /// ([`Session::not_hearing`]). The two questions differ: staffing asks *why is this
    /// position unanswered*, of people who are meant to be on the loop, and this asks *does
    /// this person think they are listening*. So an operator who stepped away from a loop they
    /// never subscribed to is in the third bucket here and `off console` there, and both are
    /// right.
    ///
    /// Nothing where the grid does not let this session near any of the armed loops.
    ///
    /// [ADR-0034]: ../../docs/adr/0034-the-transmit-bar-is-always-visible-and-the-audience-is-a-count.md
    fn in_the_audience_for(
        &self,
        armed_on: &[LoopId],
        by: Ladder,
        now: Instant,
    ) -> Option<InTheAudience> {
        let mut monitors_one = false;
        let mut in_reach_of_one = false;

        for armed in armed_on {
            if self.not_hearing(armed, by, now).is_none() {
                return Some(InTheAudience::Hearing);
            }

            monitors_one |= self.monitors(armed);
            in_reach_of_one |= self.reach.iter().any(|within| &within.id == armed);
        }

        match (monitors_one, in_reach_of_one) {
            (true, _) => Some(InTheAudience::PresentNotHearing),
            (false, true) => Some(InTheAudience::NotSubscribed),
            (false, false) => None,
        }
    }

    /// How loud that loop plays in this operator's ears. Unity for a loop they have not set.
    fn volume_of(&self, held_on: &LoopId) -> Volume {
        self.volumes
            .iter()
            .find(|(set_on, _)| set_on == held_on)
            .map_or(Volume::UNITY, |(_, volume)| *volume)
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
        let held = self.arms.len();
        self.arms.retain(|armed| {
            reach
                .iter()
                .any(|within| &within.id == armed && within.permission.carries(Permission::Emit))
        });

        // **Provenance costs one flag on the update** ([ADR-0058]). This is the one change to
        // an arm set in v1 that no hand on the operator's desk made, and the operator may be
        // mid-sentence when it lands — so the fact that it was somebody else is recorded here,
        // where the change is made, rather than inferred later from a set that moved.
        //
        // It is only ever raised, never lowered: a deliberate act is what answers it, and an
        // arm this revocation left alone is not that.
        //
        // [ADR-0058]: ../../docs/adr/0058-the-transmit-bar-is-live-while-keyed.md
        self.arms_moved_elsewhere |= self.arms.len() != held;
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

/// A priority press, held while the key is down.
///
/// The two facts the audit entry needs from the moment of the press are taken **then**,
/// because by the release they may have moved: the arm set can change under a held key, and
/// the entry is about the set the priority was keyed over (v1 §12).
struct Pressing {
    began: Instant,
    /// The wall-clock time of the press, which is what the log is read by.
    at: SystemTime,
    /// The armed loops at the moment of the press, by name as the grid had them.
    armed_on: Vec<String>,
}

/// A priority press that has ended, named well enough to audit (v1 §12).
///
/// **Every press is one of these, with no minimum duration** ([ADR-0046]). A 200 ms fumble is
/// still a decision that defeated everyone's volume setting, and abuse may look like a hundred short
/// jabs, so filtering belongs to whoever reads the log and nothing here decides what was too
/// short to count.
///
/// It is handed back by whatever ended the press — the client letting go, the channel going,
/// the session ending — and the audit entry is written by the caller, because the live side
/// writes nothing durable ([ADR-0039]).
///
/// [ADR-0039]: ../../docs/adr/0039-live-state-is-in-process-behind-one-state-authority.md
/// [ADR-0046]: ../../docs/adr/0046-priority-is-keyed-not-held.md
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pressed {
    pub(crate) occupant: UserId,
    pub(crate) role: RoleId,
    pub(crate) armed_on: Vec<String>,
    pub(crate) at: SystemTime,
    pub(crate) lasted: Duration,
}

/// Everything live, behind one lock so there is one writer.
#[derive(Default)]
struct Live {
    /// The four timers the connection ladder is read against, fixed at startup and never
    /// re-read ([ADR-0018], v1 §7).
    ///
    /// It is a value handed in at construction rather than something asked for per call,
    /// because it is neither live state nor durable state: it is what this process was
    /// started with, and every rung read anywhere in here has to be read against the same
    /// one. The same rule the rest of this module holds still holds — nothing durable is
    /// read here.
    ///
    /// [ADR-0018]: ../../docs/adr/0018-no-signalling-channel-means-no-emission-path.md
    ladder: Ladder,
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
    /// Who counts which beacon, as it was last taken away to be executed — the same device
    /// as `last_routing`, for the same reason.
    last_beacons: Option<Vec<WhoCounts>>,
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
    /// The volumes this pair last had, as Configuration remembers them — one per loop
    /// somebody set, and every other loop at unity.
    ///
    /// **There is no mute beside it**, and that is [ADR-0050] rather than a gap: a stale mute
    /// silences a loop before its owner has looked at anything, so nothing remembers one and
    /// there is nothing to hand in.
    ///
    /// [ADR-0050]: ../../docs/adr/0050-personalisation-persists-what-is-safe-to-be-stale.md
    pub(crate) volumes: Vec<(LoopId, Volume)>,
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
    /// The priority press that was held when the session ended, where one was.
    ///
    /// The session ending ends the press, and the press still happened: relinquishing under a
    /// held priority key is not a way to keep it out of the log (v1 §12).
    pub(crate) pressed: Option<Pressed>,
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
    /// Whether a transmission on this loop right now is at priority.
    ///
    /// **The talking indicator's one variant, and it is not attribution** ([ADR-0046]): it says
    /// what *kind* of transmission is on the loop and never whose, and like `talking` it is one
    /// flag however many talkers there are. It is in every document whose reach holds the loop
    /// — monitored or not, muted or not, at full volume or not — because it is **a declaration
    /// that somebody called this urgent** rather than an explanation of a gain change
    /// ([ADR-0059]). It lives exactly as long as the press, with no floor.
    ///
    /// It is also what the receiving client plays at full gain from ([ADR-0045]), so the mark
    /// and the gain are one fact arriving and resolve together.
    ///
    /// [ADR-0045]: ../../docs/adr/0045-priority-defeats-attenuation-and-nothing-else.md
    /// [ADR-0046]: ../../docs/adr/0046-priority-is-keyed-not-held.md
    /// [ADR-0059]: ../../docs/adr/0059-a-priority-transmission-is-marked-wherever-it-lands.md
    pub(crate) priority: bool,
    /// Whether this session has silenced this loop in its own ears.
    ///
    /// **Only ever true beside `subscribed`**, because a mute presupposes a subscription
    /// ([ADR-0049]). It is its own field rather than a third value of that one because the two
    /// are read for different things: the subscription is what keeps the talking indicator
    /// and loop health arriving, and the mute is what stops the audio.
    ///
    /// [ADR-0049]: ../../docs/adr/0049-the-role-is-the-profile.md
    pub(crate) muted: bool,
    /// How loud this loop plays in this operator's ears.
    ///
    /// It is in the document because the document is the API and the console shows it —
    /// and because **per-loop volume is the one attenuation in VoxLoop that nothing warns
    /// anybody about** (v1 §4). The card saying so is the only place the operator who turned
    /// it down is reminded.
    pub(crate) volume: Volume,
    /// Whether this session is actually receiving this loop, measured from its beacon
    /// ([ADR-0017]).
    ///
    /// **Nothing on a loop this session is not monitoring**, because there is no beacon to
    /// count, and **nothing while its signalling channel is not confirmed**, because connection
    /// state already explains the silence (v1 §6). It is carried on a muted loop like the
    /// talking indicator is.
    ///
    /// [ADR-0017]: ../../docs/adr/0017-loop-health-is-measured-not-asserted.md
    pub(crate) health: Option<LoopHealth>,
    /// Whether a human is behind this loop, or nothing where it has no staffing roles
    /// ([ADR-0056]).
    ///
    /// It is a fact about the **loop** rather than about this session, and it is in every
    /// document whose reach holds the loop: what it answers — *is somebody covering this
    /// position* — is asked by whoever is about to key, not by whoever is staffing it.
    ///
    /// The absence is not a fourth value and never renders as one. A loop that loses its
    /// last staffing role moves from a state to nothing, which is a legitimate
    /// configuration change arriving mid-session like any other.
    ///
    /// [ADR-0056]: ../../docs/adr/0056-a-loop-with-no-staffing-roles-has-no-staffing-state.md
    pub(crate) staffing: Option<Staffing>,
    /// Whether **this session's role** staffs this loop.
    ///
    /// The mark the console draws in both views, in two states — *you staff this*, and *you
    /// would staff this and are not subscribed* (v1 §8). Only the first half is here: the
    /// second is this field beside `subscribed`, derived on the client from the document it
    /// already has rather than computed twice ([ADR-0065]).
    ///
    /// It is carried whether or not anything is wrong, because it is a fact about the
    /// operator's own console rather than an alarm — a mark appearing for the first time at
    /// the moment something is wrong is the rendering that decision rejects.
    ///
    /// [ADR-0065]: ../../docs/adr/0065-the-staffing-flag-reports-it-never-subscribes.md
    pub(crate) staffs: bool,
}

/// One listener and every loop whose beacon it counts.
///
/// **The beacon is carried to every subscriber** ([ADR-0017]), and this is who they are: the
/// loops each session monitors within its reach, muted or not. It is computed here and
/// executed by the media plane, like the fan-out ([ADR-0063]), and it is per listener rather
/// than per loop because a loop nobody monitors still runs its beacon — the beacon is the
/// loop's and never waits for somebody to count it.
///
/// [ADR-0017]: ../../docs/adr/0017-loop-health-is-measured-not-asserted.md
/// [ADR-0063]: ../../docs/adr/0063-the-media-plane-executes-routing-it-never-computes-it.md
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WhoCounts {
    pub(crate) listener: SessionId,
    pub(crate) on: Vec<LoopId>,
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

/// The one asserted state, as it stands: the claim, and how old the evidence behind it is.
///
/// It is a pair rather than a flag because **asserted state is never shown alone**
/// ([ADR-0016]). A user said they were off console at some moment; what makes that honest
/// rather than misleading is the second half — how long ago they last did anything
/// deliberate — and a type that could carry the first without the second would leave the
/// console free to omit it.
///
/// **A stale assertion is still one**, and it is still shown, with its age. VoxLoop does not
/// resolve the ambiguity of somebody who walked away and never said so; it makes the
/// ambiguity visible and leaves the judgement with the human.
///
/// [ADR-0016]: ../../docs/adr/0016-displayed-state-is-observed-or-asserted.md
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Asserted {
    /// How long ago the claimant last did anything deliberate.
    pub(crate) last_active: Duration,
}

/// Who would actually hear an emission, counted (v1 §6, [ADR-0034]).
///
/// **Three buckets, and the split is the point.** A subscriber list that silently includes
/// people who cannot hear you answers *who chose to listen* when the operator asked *who will
/// hear me*, and the two answers come apart exactly when it matters: a mute, a colleague who
/// stepped away, a console nobody can reach, a loop whose beacon is not arriving.
///
/// **It is computed and stored nowhere.** Like staffing state and the document itself, it is
/// worked out from the live facts each time it is asked for, which is what keeps it from being
/// a cached answer about a headset somebody has since pulled out.
///
/// **It is per person, never per destination.** The fan-out names a listener once per loop it
/// reaches them on, because the recording tap is per (talker, destination) ([ADR-0009]); the
/// bar answers *how many people will hear me*, and one colleague on three armed loops is one
/// person. Hearing any one of the armed loops is hearing.
///
/// [ADR-0009]: ../../docs/adr/0009-recording-taps-plain-rtp-on-loopback.md
/// [ADR-0034]: ../../docs/adr/0034-the-transmit-bar-is-always-visible-and-the-audience-is-a-count.md
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Audience {
    /// People who would hear this emission: the loop is on their console, unmuted, its beacon
    /// is arriving, they have not stepped away and they can be reached.
    ///
    /// **The reassurance**, and the count that renders in the warning colour at zero — where
    /// it blocks nothing, because the console does not overrule an operator about their own
    /// operation ([ADR-0034]).
    ///
    /// [ADR-0034]: ../../docs/adr/0034-the-transmit-bar-is-always-visible-and-the-audience-is-a-count.md
    pub(crate) hearing: usize,
    /// People who took one of these loops up and will not hear it: muted, off console,
    /// unreachable, or not receiving its beacon.
    ///
    /// **The warning, and the console's only one about mute** ([ADR-0034]). It is the count
    /// that says *these people believe they are covering this loop and will not hear you*, and
    /// it is the compensating signal for mute defeating an announcement, a directive and a
    /// hail.
    ///
    /// [ADR-0034]: ../../docs/adr/0034-the-transmit-bar-is-always-visible-and-the-audience-is-a-count.md
    pub(crate) present_not_hearing: usize,
    /// People in reach of one of these loops who did not take it up.
    ///
    /// **Computed and never displayed.** It is not actionable — a loop exists precisely so
    /// that an emitter can stop tracking who is on the other end — and a third count beside
    /// the other two would be read as another flavour of the warning. It is computed anyway
    /// because the three-way split is what makes the second bucket mean anything: without it,
    /// *not hearing* would swallow everybody who simply chose not to listen.
    ///
    /// It surfaces in exactly one place, and not as a number: the hail picker, where hailing
    /// is what makes these people actionable ([ADR-0048]).
    ///
    /// [ADR-0048]: ../../docs/adr/0048-the-hail-picker-is-the-only-place-the-console-names-a-person.md
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) not_subscribed: usize,
}

/// Which of the three a person is in, for one arm set.
///
/// Nothing at all where the grid does not let them near any of the armed loops: the buckets
/// partition the people **in reach**, and somebody who was never asked is not somebody who
/// chose not to listen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InTheAudience {
    Hearing,
    PresentNotHearing,
    NotSubscribed,
}

/// The presence document: everything one session may see, as of one moment.
///
/// It is a **projection** rather than a record. Nothing here is stored and read back — it is
/// computed from the live facts and the reach handed in, which is what lets the whole of it
/// be true at the same instant rather than assembled from several that were each true at
/// some point ([ADR-0019]).
///
/// What it carries is the session, the role it is bound to, the loops in reach and which of
/// them the session is monitoring, with the arms (#41), the talking indicator, loop health
/// (#46) and staffing state (#48) beside each — and, about the session as a whole, the
/// audience of its arm set and whether anything but this session last moved that set (#49).
/// Each of them is a field the server has committed to keeping true.
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
    /// Where the **server** has this session standing with the signalling channel
    /// ([ADR-0018]).
    ///
    /// It is deliberately not the console's own reading, and the console does not replace its
    /// reading with this one: **they are two facts and they merge pessimistically**, which is
    /// the rule already used for the media path's two ends ([ADR-0042]) — green needs both,
    /// red needs one.
    ///
    /// The two can honestly disagree, because the two ends measure different silences. This
    /// end measures the answers it is not getting; the console measures the heartbeats it is
    /// not getting. A console whose replies are lost while the server's still arrive is the
    /// case that needs this field: the server reaches the disconnect threshold and closes the
    /// fan-out, and without being told the console would go on offering a key control over a
    /// route that is already closed — which is [ADR-0008]'s residual arriving as a feature.
    ///
    /// It cannot always arrive, and that is why the console runs its own clock rather than
    /// waiting for this. A session that hears nothing is told nothing, by construction. What
    /// this covers is the half of the failure where the server can still be heard.
    ///
    /// **The age is not here.** A running age would move the document's version five times a
    /// second, and the version answers *is this the same state* (v1 §6). The console has its
    /// own clock and the age is that clock's.
    ///
    /// [ADR-0008]: ../../docs/adr/0008-emission-is-armed-by-the-server-and-keyed-by-the-client.md
    /// [ADR-0018]: ../../docs/adr/0018-no-signalling-channel-means-no-emission-path.md
    /// [ADR-0042]: ../../docs/adr/0042-the-media-path-has-its-own-ladder.md
    pub(crate) connection: Connection,
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
    /// Whether the server has this session's transmission down as at priority.
    ///
    /// The lamp's other half. **An elevated transmission shows as elevated with no new
    /// surface** ([ADR-0046]): the operator's own lamp says so, lit by this answer and never by
    /// the priority control going down.
    ///
    /// [ADR-0046]: ../../docs/adr/0046-priority-is-keyed-not-held.md
    pub(crate) priority: bool,
    /// What this operator has claimed about themselves, where they have claimed anything.
    ///
    /// **The only asserted field in the document**, and the only one the server has not seen
    /// for itself ([ADR-0016]). It carries the age of its own evidence with it, so a console
    /// cannot render the claim as though it were observed without leaving the age out, and
    /// there is nowhere to leave it out from.
    ///
    /// **This is the one place a running age is in the document**, and it is the exception the
    /// connection's age is not. The connection's age belongs to the console because a session
    /// that hears nothing is told nothing; this age belongs to the server because the acts it
    /// is measured from arrive here, and everyone who is later shown this claim — the audience
    /// (#49), the staffing reason (#48) — is somebody else's console, which has no clock of
    /// its own to run it on. It moves the version once a second, and only while somebody is
    /// off console: a state in which, by its own claim, nothing else on that console is moving
    /// at all.
    ///
    /// [ADR-0016]: ../../docs/adr/0016-displayed-state-is-observed-or-asserted.md
    pub(crate) off_console: Option<Asserted>,
    /// Who would actually hear this session's arm set, counted (v1 §6).
    ///
    /// It is in the document because the transmit bar renders it, and it is the whole of
    /// VoxLoop's compensation for emitting to several places at once: the receiver is told
    /// nothing about a transmission's other destinations ([ADR-0057]), so the emitter is told
    /// everything about their own.
    ///
    /// **It is here whether or not this session is keyed**, because the bar answers *who am I
    /// about to talk to* before the key goes down and *who am I talking to* under it, with the
    /// same words ([ADR-0058]).
    ///
    /// [ADR-0057]: ../../docs/adr/0057-the-receiver-is-never-told-where-else-a-transmission-went.md
    /// [ADR-0058]: ../../docs/adr/0058-the-transmit-bar-is-live-while-keyed.md
    pub(crate) audience: Audience,
    /// Whether the arm set below was last moved by something other than this session.
    ///
    /// **The mark, and the third place the console renders provenance** ([ADR-0058]), after
    /// the observed-or-asserted split and the directed subscription: a state somebody else
    /// moved must not look like one you set. A deliberate arm and a preset are not marked.
    ///
    /// [ADR-0058]: ../../docs/adr/0058-the-transmit-bar-is-live-while-keyed.md
    pub(crate) arms_moved_elsewhere: bool,
    pub(crate) loops: Vec<Standing>,
}

impl StateAuthority {
    /// A running system with nobody on it — which is what a restart leaves — keeping time by
    /// the ladder this deployment was started with.
    pub(crate) fn keeping_time_by(ladder: Ladder) -> Self {
        Self {
            live: Mutex::new(Live {
                ladder,
                ..Live::default()
            }),
        }
    }

    /// The same, on the ladder v1 §7 fixes.
    ///
    /// It is what a test runs on, and it is the same ladder a deployment that has tuned
    /// nothing runs on — so a test asking about a rung is asking about the numbers the spec
    /// names rather than about numbers of its own.
    #[cfg(test)]
    pub(crate) fn empty() -> Self {
        Self::default()
    }

    /// The four timers this deployment runs on.
    ///
    /// Transport asks for them twice: to space the heartbeats it sends, and to carry the
    /// whole ladder to the console — which runs the client's half of it, because the one
    /// thing a server cannot do to a console it has lost is tell it anything.
    pub(crate) fn ladder(&self) -> Ladder {
        self.read(|live| live.ladder)
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
            let now = Instant::now();
            live.sessions.push(Session {
                id: session.clone(),
                sign_in: assuming.sign_in,
                occupant: assuming.occupant,
                role: assuming.role,
                version: 0,
                last: None,
                said_by_the_client: MediaPath::default(),
                seen_by_the_server: MediaPath::default(),
                // Every loop restored is a loop whose beacon has not been counted yet.
                beacons: assuming
                    .subscribed_to
                    .iter()
                    .map(|held_on| Counting::starting(held_on.clone(), now))
                    .collect(),
                subscriptions: assuming.subscribed_to,
                // **Nothing is armed and nothing is keyed on a seat just taken.** The
                // subscription set is restored because it is remembered personalisation
                // (ADR-0050) and a restart otherwise costs every operator their console by
                // hand; an arm set restored the same way would put somebody on the air the
                // instant they assumed, which is why nothing remembers one.
                arms: Vec::new(),
                // Nothing has taken an arm from a set that has never held one.
                arms_moved_elsewhere: false,
                // Nor is anything muted, whatever the last session in this seat had: a mute
                // is never remembered (ADR-0050), and one restored here would silence a loop
                // before the operator had looked at anything.
                mutes: Vec::new(),
                volumes: assuming.volumes,
                keyed: false,
                pressing: None,
                heard_from: now,
                the_channel_is_gone: false,
                reach: Vec::new(),
                // **Nobody is off console on a seat just taken.** Nothing remembers an
                // assertion (ADR-0050) and nothing could honestly restore one: a day-old
                // claim about where somebody was sitting is not a fact about anything.
                off_console: false,
                // Assuming a role is the most deliberate act there is, so the clock the
                // assertion would be shown against starts full rather than at nothing.
                last_active: now,
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
                // A loop taken up has not proved it reaches anybody yet, and a count carried
                // over from the last time it was up would be a carriage that no longer exists.
                held.beacons
                    .push(Counting::starting(to.clone(), Instant::now()));
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
            held.beacons.retain(|counting| &counting.held_on != from);
            // **A mute is dropped with its subscription** (ADR-0049). It presupposes one, and
            // a mute left behind here would silence the loop the next time it was taken up.
            held.mutes.retain(|held_on| held_on != from);

            true
        })
    }

    /// Silence a loop in this operator's own ears.
    ///
    /// **Not a permission and not an unsubscribe** (v1 §5). The subscription stands, so the
    /// talking indicator, loop health and the priority mark keep arriving; what stops is the
    /// audio, because [`Session::hears`] reads the mute and the fan-out is built from that.
    /// Nobody else on the loop is touched.
    ///
    /// It is `Session` rather than a grid check (`docs/spec/api-surface.md`): it reaches
    /// nothing and nobody, so there is no rung for it to need.
    ///
    /// **A loop nobody is monitoring has nothing to mute** ([ADR-0049]), so muting one leaves
    /// nothing behind for a later subscribe to find. It is a set, for the reason every other
    /// act here is: the control lags the click, and a second press that has not caught up
    /// must land on the same state.
    ///
    /// It answers whether a live session took the act. Nothing where the id names no session.
    ///
    /// [ADR-0049]: ../../docs/adr/0049-the-role-is-the-profile.md
    pub(crate) fn mute(&self, session: &SessionId, held_on: &LoopId) -> bool {
        self.write(|live| {
            let Some(held) = live.sessions.iter_mut().find(|held| &held.id == session) else {
                return false;
            };

            if held.subscriptions.contains(held_on) && !held.mutes.contains(held_on) {
                held.mutes.push(held_on.clone());
            }

            true
        })
    }

    /// Hear a muted loop again. The other half of the act, idempotent for the same reason.
    ///
    /// **Nothing else ever does this.** A mute does not expire and a resume does not clear it
    /// (v1 §5, §7): an unexpected un-mute mid-incident is its own hazard, so the only way out
    /// of one is the operator's own hand — or dropping the loop, which takes the mute with it.
    pub(crate) fn unmute(&self, session: &SessionId, held_on: &LoopId) -> bool {
        self.write(|live| {
            let Some(held) = live.sessions.iter_mut().find(|held| &held.id == session) else {
                return false;
            };

            held.mutes.retain(|muted| muted != held_on);

            true
        })
    }

    /// Say that this operator is not in the chair.
    ///
    /// **The one asserted state in VoxLoop, and it is only ever set by the person it is about**
    /// ([ADR-0016]). There is no clock here and no idleness anywhere near it: an operator
    /// watching telemetry for twenty minutes is doing their job, and a console that demoted
    /// them would be inventing a state to be helpful with.
    ///
    /// **It changes nothing else.** Subscriptions stand, arms stand, volumes stand and the
    /// fan-out is untouched, so the operator who steps away and hears something over their
    /// headset from three metres away still hears it, and coming back is a click rather than a
    /// resynchronisation. What it costs is the staffing state of the loops this session's role
    /// staffs, which drops to `away` — computed with #48, from [`Session::not_hearing`], which
    /// already reads it.
    ///
    /// It is a **set** like every other act here: a second press on a control that has not
    /// caught up says the same thing twice and lands on the same state.
    ///
    /// It answers whether a live session took the act. Nothing where the id names no session.
    ///
    /// [ADR-0016]: ../../docs/adr/0016-displayed-state-is-observed-or-asserted.md
    pub(crate) fn off_console(&self, session: &SessionId) -> bool {
        self.write(|live| {
            let Some(held) = live.sessions.iter_mut().find(|held| &held.id == session) else {
                return false;
            };

            held.off_console = true;

            true
        })
    }

    /// Somebody did something deliberate on this session: they are in the chair, as of now.
    ///
    /// **This is the whole of how an off-console assertion is cleared** ([ADR-0016]), and
    /// there is deliberately no second way out of one. Keying, changing a subscription or an
    /// arm, answering a prompt, dismissing a banner — each is a person acting on a console,
    /// and each is unambiguous evidence of the thing the assertion denies. Saying *I am back*
    /// is one of them rather than a special case, which is why nothing here is named for it.
    ///
    /// **What is not one of them is the whole point.** Mouse movement, scroll and focus never
    /// reach this: they are the machine reporting that a page exists, and a cat on a keyboard
    /// clearing an assertion is the guessing this rule forbids, arriving through the other
    /// door. Which messages count is Transport's answer, and it is exhaustive over the
    /// protocol so that a message nobody has ruled on does not compile.
    ///
    /// It refreshes the last-active clock whether or not an assertion stands, because that
    /// clock is what the *next* assertion will be shown against.
    ///
    /// Nothing where the id names no session — a lobby socket has no chair to be in.
    ///
    /// [ADR-0016]: ../../docs/adr/0016-displayed-state-is-observed-or-asserted.md
    pub(crate) fn a_deliberate_act(&self, session: &SessionId) {
        self.write(|live| {
            if let Some(held) = live.sessions.iter_mut().find(|held| &held.id == session) {
                held.off_console = false;
                // **The mark on an arm set somebody else moved is answered the same way**
                // ([ADR-0058]): the operator has acted on this console, so they have read
                // what is on it. It is the same evidence the assertion above is cleared by,
                // and it is deliberately not a dismissal of its own.
                //
                // [ADR-0058]: ../../docs/adr/0058-the-transmit-bar-is-live-while-keyed.md
                held.arms_moved_elsewhere = false;
                held.last_active = Instant::now();
            }
        });
    }

    /// Set how loud a loop plays in this operator's ears.
    ///
    /// **Personalisation, and not a live operational control** (v1 §5): it is applied here so
    /// that it is heard at once and shown in the document, and remembered by whoever called,
    /// best effort ([ADR-0050]). It reaches nothing and nobody, so it is `Session` rather than
    /// a grid check, and a volume on a loop outside reach is kept and inert like everything
    /// else a grid edit narrows ([ADR-0051]).
    ///
    /// **It changes no route.** The fan-out does not read it; the client plays each talker at
    /// the loudest volume among the loops it hears them on ([ADR-0007]).
    ///
    /// It answers whether a live session took the act, which is what the caller needs to know
    /// before remembering it.
    ///
    /// [ADR-0007]: ../../docs/adr/0007-the-client-emits-one-stream.md
    /// [ADR-0050]: ../../docs/adr/0050-personalisation-persists-what-is-safe-to-be-stale.md
    /// [ADR-0051]: ../../docs/adr/0051-personalisation-is-scoped-to-the-smallest-thing-it-is-about.md
    pub(crate) fn set_the_volume(
        &self,
        session: &SessionId,
        held_on: &LoopId,
        volume: Volume,
    ) -> bool {
        self.write(|live| {
            let Some(held) = live.sessions.iter_mut().find(|held| &held.id == session) else {
                return false;
            };

            match held
                .volumes
                .iter_mut()
                .find(|(set_on, _)| set_on == held_on)
            {
                Some((_, was)) => *was = volume,
                None => held.volumes.push((held_on.clone(), volume)),
            }

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

    /// The client says its transmission is at priority: somebody is holding the priority key.
    ///
    /// **Available to anyone holding `emit`, and ungated** ([ADR-0046]). There is no rung to
    /// consult and no flag on the role, the loop or the cell: what the transmission may reach
    /// was settled when the arms were made, and priority governs gain and never who receives
    /// ([ADR-0045]). What it changes is what everybody is told — the mark on every armed loop —
    /// and what every listener's client plays at full gain.
    ///
    /// **It is the start of a press**, and the press is what gets audited. The arm set is taken
    /// now, as it stands, by name. A second key-down while one is held is the same press: a
    /// client that says it twice has not pressed twice.
    ///
    /// It answers whether a live session took the act.
    ///
    /// [ADR-0045]: ../../docs/adr/0045-priority-defeats-attenuation-and-nothing-else.md
    /// [ADR-0046]: ../../docs/adr/0046-priority-is-keyed-not-held.md
    pub(crate) fn the_client_keys_priority(&self, session: &SessionId) -> bool {
        self.write(|live| {
            let Some(held) = live.sessions.iter_mut().find(|held| &held.id == session) else {
                return false;
            };

            if held.pressing.is_none() {
                let armed_on = held
                    .arms
                    .iter()
                    .map(|armed| {
                        held.reach
                            .iter()
                            .find(|within| &within.id == armed)
                            .map_or_else(|| armed.as_str().to_owned(), |within| within.name.clone())
                    })
                    .collect();

                held.pressing = Some(Pressing {
                    began: Instant::now(),
                    at: SystemTime::now(),
                    armed_on,
                });
            }

            true
        })
    }

    /// The client says its transmission is no longer at priority: the key was let go.
    ///
    /// **Priority never latches** ([ADR-0046]), so this is the whole of how a press ends in the
    /// ordinary case. The transmission itself is untouched — letting go of priority over a
    /// latch or a held key lowers it and does not end it — because that is `keyed`, which the
    /// client says separately.
    ///
    /// It answers with the press that ended, to be audited, and nothing where no press was
    /// being held.
    ///
    /// [ADR-0046]: ../../docs/adr/0046-priority-is-keyed-not-held.md
    pub(crate) fn the_client_unkeys_priority(&self, session: &SessionId) -> Option<Pressed> {
        self.write(|live| {
            live.sessions
                .iter_mut()
                .find(|held| &held.id == session)?
                .let_go()
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
            let now = Instant::now();
            let routing: Vec<WhoHears> = live
                .sessions
                .iter()
                .map(|talker| WhoHears {
                    talker: talker.id.clone(),
                    listeners: live.who_hears(talker, now),
                })
                .collect();

            if live.last_routing.as_ref() == Some(&routing) {
                return None;
            }
            live.last_routing = Some(routing.clone());

            Some(routing)
        })
    }

    /// Who counts which beacon, where that has moved since it was last taken.
    ///
    /// **Every subscriber consumes the beacon of every loop it monitors** ([ADR-0017]), muted or
    /// not, within its reach. It is computed here and executed there, like the fan-out
    /// ([ADR-0063]), and for the same reasons it moves only when it moves and is taken rather
    /// than read.
    ///
    /// **Nothing here decides which loops run a beacon.** Every loop does, whether or not
    /// anybody counts it — suppressing one would make the mechanism unavailable at exactly the
    /// moment somebody subscribes — and the list of loops is Configuration's, which this module
    /// does not read.
    ///
    /// [ADR-0017]: ../../docs/adr/0017-loop-health-is-measured-not-asserted.md
    /// [ADR-0063]: ../../docs/adr/0063-the-media-plane-executes-routing-it-never-computes-it.md
    pub(crate) fn the_beacons_if_they_moved(&self) -> Option<Vec<WhoCounts>> {
        self.write(|live| {
            let counting: Vec<WhoCounts> = live
                .sessions
                .iter()
                .map(|listener| WhoCounts {
                    listener: listener.id.clone(),
                    on: listener
                        .subscriptions
                        .iter()
                        .filter(|held_on| listener.monitors(held_on))
                        .cloned()
                        .collect(),
                })
                .collect();

            if live.last_beacons.as_ref() == Some(&counting) {
                return None;
            }
            live.last_beacons = Some(counting.clone());

            Some(counting)
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
        staffed: &[StaffedBy],
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
            let now = Instant::now();
            let spoken_on = live.the_loops_being_spoken_on(now);
            let at_priority = live.the_loops_spoken_on_at_priority(now);
            // Every occupant of every staffing role, which is every session on the
            // deployment rather than this one — and worked out after this session's reach
            // was recorded, because what an occupant is hearing is read within their own.
            let staffing = live.the_staffing_of(staffed, now);
            // The same shape again, and for the same reason: the audience of this arm set is
            // read off every other session's console, so it is worked out while every one of
            // them can be seen and before this one is borrowed to be written into.
            let audience = live.the_audience_of(session, now);
            let ladder = live.ladder;

            let held = live.sessions.iter_mut().find(|held| &held.id == session)?;
            let presence = Presence {
                session: held.id.clone(),
                role: held.role.clone(),
                media_path: held.media_path(),
                connection: held.connection(ladder, now),
                keyed: held.is_transmitting(ladder, now),
                priority: held.is_at_priority(ladder, now),
                off_console: held.asserted(now),
                audience,
                arms_moved_elsewhere: held.arms_moved_elsewhere,
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
                        priority: at_priority.contains(&held_on.id),
                        muted: held.mutes.contains(&held_on.id),
                        volume: held.volume_of(&held_on.id),
                        health: held.health_of(&held_on.id, ladder, now),
                        staffing: staffing
                            .iter()
                            .find(|(loop_id, _)| loop_id == &held_on.id)
                            .map(|(_, staffing)| staffing.clone()),
                        // The loop is marked on this console because this session's role is
                        // one of the roles staffing it. Nothing about what anybody is
                        // hearing comes into it: the mark is a fact about the configuration
                        // this operator sat down into.
                        staffs: staffed.iter().any(|staffed| {
                            staffed.held_on == held_on.id && staffed.roles.contains(&held.role)
                        }),
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

    /// The client answered a heartbeat: this session's channel is confirmed as of now.
    ///
    /// **It is the only thing that moves the ladder back down**, and it says nothing about
    /// anything else. A heartbeat is the machine noticing it can still be reached, not a
    /// person doing something, so it clears no assertion, refreshes no sign-in and is not
    /// evidence that anybody is in the chair (v1 §6).
    ///
    /// Nothing where the id names no session, which is what a client answering into a session
    /// that ended under it finds.
    ///
    /// **A channel coming back starts every beacon's measurement again.** The counts ride this
    /// channel and stopped when it did, so a beacon last counted before the gap says nothing
    /// about now in either direction, and reading it as lost would put a reason on every loop
    /// for the second it takes the next count to arrive.
    pub(crate) fn the_client_is_there(&self, session: &SessionId) {
        self.write(|live| {
            let ladder = live.ladder;
            if let Some(held) = live.sessions.iter_mut().find(|held| &held.id == session) {
                let now = Instant::now();
                if held.connection(ladder, now) != Connection::Confirmed {
                    for counting in &mut held.beacons {
                        counting.again(now);
                    }
                }

                held.heard_from = now;
                held.the_channel_is_gone = false;
            }
        });
    }

    /// What this session's client has counted of each beacon it is carried ([ADR-0017]).
    ///
    /// **Loop health is measured from this and from nothing else.** The client counts the
    /// packets arriving on each beacon's carriage and says the running totals; a total that
    /// moved is a beacon that crossed the same transport, router and fan-out that speech
    /// would. A loop this session is not monitoring has nothing to measure, so a count for one
    /// is not taken.
    ///
    /// It is a report and not an act: it clears nothing, confirms nothing about the channel,
    /// and is not evidence that anybody is in the chair (v1 §6).
    ///
    /// [ADR-0017]: ../../docs/adr/0017-loop-health-is-measured-not-asserted.md
    pub(crate) fn the_client_counted(&self, session: &SessionId, counts: &[(LoopId, u64)]) {
        self.write(|live| {
            let Some(held) = live.sessions.iter_mut().find(|held| &held.id == session) else {
                return;
            };

            let now = Instant::now();
            for (held_on, packets) in counts {
                if let Some(counting) = held
                    .beacons
                    .iter_mut()
                    .find(|counting| &counting.held_on == held_on)
                {
                    counting.counted(*packets, now);
                }
            }
        });
    }

    /// Why this occupant is not hearing that loop, or nothing where they are.
    ///
    /// **It is the one fact about each occupant that staffing state is built from** (#48), and
    /// beacon loss is in it: a loop whose staffing-role occupants are all failing to receive it
    /// reads `away` (ADR-0017). Nothing where the id names no session.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn why_not_hearing(
        &self,
        session: &SessionId,
        held_on: &LoopId,
    ) -> Option<NotHearing> {
        let now = Instant::now();

        self.read(|live| {
            live.sessions
                .iter()
                .find(|held| &held.id == session)?
                .not_hearing(held_on, live.ladder, now)
        })
    }

    /// Wind back when each of this session's beacons was last counted, and when each began to
    /// be measured, so that a test can stand past the window without waiting for it.
    ///
    /// The same kind of wound-on clock as [`StateAuthority::unheard_from_for`]: it moves the
    /// facts health is measured from and nothing else, so the reading is still derived by the
    /// arithmetic the product runs.
    #[cfg(test)]
    pub(crate) fn the_beacons_went_unheard_for(&self, session: &SessionId, ago: Duration) {
        self.write(|live| {
            if let Some(held) = live.sessions.iter_mut().find(|held| &held.id == session) {
                for counting in &mut held.beacons {
                    counting.since -= ago;
                    counting.arrived = counting.arrived.map(|at| at - ago);
                }
            }
        });
    }

    /// The signalling channel for this session has gone, and the server knows it rather than
    /// merely failing to hear it.
    ///
    /// **It does not end the session** ([ADR-0041]): occupancy survives the loss of the
    /// channel and is held for the reconnection window, so this moves one axis and nothing
    /// else. What it does move is immediate — a closed socket is a fact, not a silence, so
    /// there is nothing to wait out and the fan-out closes on the next turn.
    ///
    /// **It ends a priority press** and hands it back to be audited. A priority key held across
    /// an outage is suppressed until released ([ADR-0043]), so nothing is left standing to
    /// raise anybody's volume when a new socket comes back — and the socket that carried the
    /// release is the one that has gone, so the release will never arrive to end it otherwise.
    ///
    /// [ADR-0041]: ../../docs/adr/0041-a-session-is-resumed-by-name.md
    /// [ADR-0043]: ../../docs/adr/0043-a-resume-restores-everything-except-the-key.md
    pub(crate) fn the_channel_is_gone(&self, session: &SessionId) -> Option<Pressed> {
        self.write(|live| {
            let held = live.sessions.iter_mut().find(|held| &held.id == session)?;
            held.the_channel_is_gone = true;

            held.let_go()
        })
    }

    /// Push a session's last heartbeat back by `ago`, so that a test can stand at a rung
    /// without waiting for the clock to reach it.
    ///
    /// It moves exactly the one fact the ladder is measured from and nothing else, which is
    /// what keeps it a wound-on clock rather than a way of setting a rung by hand: the rungs
    /// themselves are still derived, still by the same arithmetic the product runs, and still
    /// against the ladder v1 §7 fixes.
    #[cfg(test)]
    pub(crate) fn unheard_from_for(&self, session: &SessionId, ago: Duration) {
        self.write(|live| {
            if let Some(held) = live.sessions.iter_mut().find(|held| &held.id == session) {
                held.heard_from = Instant::now() - ago;
            }
        });
    }

    /// Push a session's last deliberate act back by `ago`, so that a test can stand at an age
    /// without waiting for the clock to reach it.
    ///
    /// It winds the one fact the age is measured from, exactly as [`StateAuthority::unheard_from_for`]
    /// winds the one the ladder is measured from. **It asserts nothing**: a session wound back
    /// an hour is a session nobody has touched for an hour, which is not the same statement as
    /// being off console and is not written like one.
    #[cfg(test)]
    pub(crate) fn last_active_was(&self, session: &SessionId, ago: Duration) {
        self.write(|live| {
            if let Some(held) = live.sessions.iter_mut().find(|held| &held.id == session) {
                held.last_active = Instant::now() - ago;
            }
        });
    }

    /// Where a session stands with the signalling channel, now.
    ///
    /// It is deliberately **not** in the presence document: a session at `disconnected` is by
    /// definition one nothing can be delivered to, so a field telling it so would be the one
    /// field in the document that could never arrive when it mattered. The console runs its
    /// own half of the ladder off the heartbeats it is missing.
    ///
    /// So nothing above this seam reads a rung yet, and the rung is not offered to anything
    /// that does not: the fan-out and the talking indicator are read off it inside this
    /// module. **It goes on a surface with #48**, where the loops a disconnected session
    /// staffs drop to `away — unreachable` with the age, and it stops being a test-only
    /// answer then.
    #[cfg(test)]
    pub(crate) fn connection_of(&self, session: &SessionId) -> Option<Connection> {
        let now = Instant::now();

        self.read(|live| {
            live.sessions
                .iter()
                .find(|held| &held.id == session)
                .map(|held| held.connection(live.ladder, now))
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

    /// Whether a human is behind each of these loops (v1 §1).
    ///
    /// The live half of staffing state. Which roles staff which loops is Configuration's and
    /// arrives as a value; who occupies those roles and what they are hearing is this
    /// module's, and the two meet here by the rule they always meet by ([ADR-0039]).
    ///
    /// The loops handed in are the ones with staffing roles, so every one of them has an
    /// answer. A loop that is not in the list has no staffing state at all, and nothing here
    /// invents one for it ([ADR-0056]).
    ///
    /// It is the same computation the presence document carries — one function, called from
    /// the two places that need it, so the lobby and the console cannot come to disagree
    /// about whether somebody is behind a loop.
    ///
    /// [ADR-0039]: ../../docs/adr/0039-live-state-is-in-process-behind-one-state-authority.md
    /// [ADR-0056]: ../../docs/adr/0056-a-loop-with-no-staffing-roles-has-no-staffing-state.md
    pub(crate) fn the_staffing_of(&self, staffed: &[StaffedBy]) -> Vec<(LoopId, Staffing)> {
        let now = Instant::now();

        self.read(|live| live.the_staffing_of(staffed, now))
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
fn ended(mut session: Session, why: Ended) -> Relinquished {
    let pressed = session.let_go();

    Relinquished {
        session: session.id,
        sign_in: session.sign_in,
        occupant: session.occupant,
        role: session.role,
        why,
        pressed,
    }
}

impl Live {
    /// The staffing state of each of these loops, computed over every occupant of every
    /// role that staffs it (v1 §1).
    ///
    /// **There is no partial value.** One occupant hearing it makes the loop `staffed`
    /// however many others are not, because the question is *is a human behind this loop*
    /// and one is. Where none of them is, every one of them is counted, because occupants
    /// can be away for different reasons at once and **no ordering across people is
    /// defensible** — a mute is one click from hearing and so is a subscription ([ADR-0065]).
    /// Within one occupant the reason is the one furthest upstream, which
    /// [`Session::not_hearing`] decides.
    ///
    /// Nothing is stored: like the audience and the document, it is worked out from the
    /// live facts each time it is asked for.
    ///
    /// [ADR-0065]: ../../docs/adr/0065-the-staffing-flag-reports-it-never-subscribes.md
    fn the_staffing_of(&self, staffed: &[StaffedBy], now: Instant) -> Vec<(LoopId, Staffing)> {
        staffed
            .iter()
            .map(|staffing| (staffing.held_on.clone(), self.staffing_of(staffing, now)))
            .collect()
    }

    fn staffing_of(&self, staffing: &StaffedBy, now: Instant) -> Staffing {
        let occupants = self
            .sessions
            .iter()
            .filter(|held| staffing.roles.contains(&held.role));

        let mut counted = [0usize; NotHearing::FURTHEST_UPSTREAM_FIRST.len()];
        let mut anybody = false;
        for occupant in occupants {
            anybody = true;
            match occupant.not_hearing(&staffing.held_on, self.ladder, now) {
                // Demonstrably hearing it, so the loop is staffed and the rest of the
                // occupants change nothing about that.
                None => return Staffing::Staffed,
                Some(reason) => {
                    let at = NotHearing::FURTHEST_UPSTREAM_FIRST
                        .iter()
                        .position(|upstream| *upstream == reason)
                        .expect("every reason is in the order it is chosen in");
                    counted[at] += 1;
                }
            }
        }

        if !anybody {
            return Staffing::Vacant;
        }

        Staffing::Away(
            NotHearing::FURTHEST_UPSTREAM_FIRST
                .into_iter()
                .zip(counted)
                .filter(|(_reason, occupants)| *occupants > 0)
                .collect(),
        )
    }

    /// The audience of this session's arm set: who would actually hear it, counted (v1 §6).
    ///
    /// **It is a projection over every session at once**, which is why it is here rather than
    /// on the session: what one operator's audience looks like is decided by what everybody
    /// else has on their console, and nothing durable is read to work it out ([ADR-0039]).
    ///
    /// **A person is counted once, in one bucket**, and a user occupies at most one role at a
    /// time — so a session is a person, and per (role, user) and per session are the same
    /// count.
    ///
    /// **The talker is not in their own audience.** Hearing yourself back over the network is
    /// a fault in an intercom, and counting yourself is that fault arriving as a number.
    ///
    /// **Nothing here asks whether the talker is keyed, or whether their own paths are up.**
    /// The answer is the same one before the key goes down and under it ([ADR-0058]), and a
    /// count that collapsed when the talker's own channel went would be answering a question
    /// about this console with a fact about somebody else's. What the operator's own
    /// withdrawal costs them is said in words, beside the key control.
    ///
    /// [ADR-0039]: ../../docs/adr/0039-live-state-is-in-process-behind-one-state-authority.md
    /// [ADR-0058]: ../../docs/adr/0058-the-transmit-bar-is-live-while-keyed.md
    fn the_audience_of(&self, talker: &SessionId, now: Instant) -> Audience {
        let Some(talking) = self.sessions.iter().find(|held| &held.id == talker) else {
            return Audience::default();
        };

        let mut audience = Audience::default();
        for listener in self.sessions.iter().filter(|held| held.id != talking.id) {
            match listener.in_the_audience_for(&talking.arms, self.ladder, now) {
                Some(InTheAudience::Hearing) => audience.hearing += 1,
                Some(InTheAudience::PresentNotHearing) => audience.present_not_hearing += 1,
                Some(InTheAudience::NotSubscribed) => audience.not_subscribed += 1,
                None => {}
            }
        }

        audience
    }

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
    /// **A talker with no signalling channel reaches nobody**, and that is the server's half
    /// of ADR-0018 rather than a second reading of the arm set. Disabling push-to-talk in the
    /// client is not sufficient: the situation that motivates the rule is precisely the one
    /// where the client may be wedged, and a wedged client's audio is still arriving at a
    /// perfectly healthy media transport. So the route is not built, using the machinery
    /// revocation and Cut already use. The arms themselves are left standing — the operator
    /// chose them, nothing has taken the rung away, and they are what the console comes back
    /// to.
    ///
    /// It is done here rather than by clearing the key, because the fan-out is per arm rather
    /// than per key: closing it means having no destinations, and that is this answer.
    ///
    /// **The loop is not told the transmission was cut, and [ADR-0018] says it should be** —
    /// with the reason *signalling lost*, so that listeners hear a voice cut rather than
    /// vanish. Nothing in v1 tells a loop anything about a transmission ending yet: the
    /// machinery is [ADR-0014]'s Cut, which is not built. It is recorded here rather than
    /// left to be discovered, and it lands with Cut.
    ///
    /// [ADR-0014]: ../../docs/adr/0014-authority-acts-on-emission-are-transient.md
    ///
    /// [ADR-0008]: ../../docs/adr/0008-emission-is-armed-by-the-server-and-keyed-by-the-client.md
    /// [ADR-0051]: ../../docs/adr/0051-personalisation-is-scoped-to-the-smallest-thing-it-is-about.md
    fn who_hears(&self, talker: &Session, now: Instant) -> Vec<Heard> {
        if !talker.connection(self.ladder, now).carries_emission() {
            return Vec::new();
        }

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
    /// **A session with no signalling channel is not spoken on**, for the same reason its
    /// fan-out is closed: the audio has nowhere to go, so an indicator saying otherwise would
    /// be the console reporting a transmission nobody is receiving.
    ///
    /// [ADR-0033]: ../../docs/adr/0033-the-console-shows-that-someone-is-talking-never-who.md
    fn the_loops_being_spoken_on(&self, now: Instant) -> Vec<LoopId> {
        self.sessions
            .iter()
            .filter(|held| held.is_transmitting(self.ladder, now))
            .flat_map(|held| held.arms.iter().cloned())
            .collect()
    }

    /// Every loop somebody is armed and keyed on **at priority**, deployment-wide.
    ///
    /// The whole arm set of every talker at priority, because priority is an attribute of the
    /// transmission rather than of a destination ([ADR-0045]). Like the loops being spoken on,
    /// it answers with loops rather than talkers, so nothing in it could say whose.
    ///
    /// [ADR-0045]: ../../docs/adr/0045-priority-defeats-attenuation-and-nothing-else.md
    fn the_loops_spoken_on_at_priority(&self, now: Instant) -> Vec<LoopId> {
        self.sessions
            .iter()
            .filter(|held| held.is_at_priority(self.ladder, now))
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
            volumes: Vec::new(),
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
        assert!(live.presence(&assumed.session, Vec::new(), &[]).is_none());
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
            .presence(&assumed.session, vec![a_loop("air-to-ground")], &[])
            .expect("a document for a live session");

        assert_eq!(version, 1);
        assert_eq!(presence.session, assumed.session);
        assert_eq!(presence.role, role);
        assert_eq!(
            presence.loops,
            vec![Standing {
                held_on: a_loop("air-to-ground"),
                // Nothing was remembered and nothing has been taken up, so the loop is on
                // the console, not being heard, not a destination and quiet — and at unity,
                // which is where every loop starts (v1 §10). It has no health, because there is
                // no beacon being counted on a loop nobody here monitors.
                subscribed: false,
                armed: false,
                talking: false,
                priority: false,
                muted: false,
                volume: Volume::UNITY,
                health: None,
                // Nothing staffs it, so it has no staffing state and this session's role
                // does not answer for it (ADR-0056).
                staffing: None,
                staffs: false,
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
            .presence(&assumed.session, vec![a_loop("air-to-ground")], &[])
            .expect("a document");
        let (again, _) = live
            .presence(&assumed.session, vec![a_loop("air-to-ground")], &[])
            .expect("a document");
        let (moved, _) = live
            .presence(
                &assumed.session,
                vec![a_loop("air-to-ground"), a_loop("flight-director")],
                &[],
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

        live.presence(&one.session, vec![a_loop("air-to-ground")], &[])
            .expect("a document");
        live.presence(&one.session, Vec::new(), &[])
            .expect("a document");
        let (theirs, _) = live
            .presence(&two.session, vec![a_loop("air-to-ground")], &[])
            .expect("a document");

        assert_eq!(theirs, 1, "one session's version counted the other's");
    }

    // ---- #39: subscription -------------------------------------------------------------

    /// Which loops a document says this session is monitoring, by name.
    fn monitoring(live: &StateAuthority, session: &SessionId, within: Vec<InReach>) -> Vec<String> {
        live.presence(session, within, &[])
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
            .presence(&assumed.session, reach.clone(), &[])
            .expect("a document");

        live.subscribe(&assumed.session, &a_loop("flight").id);
        let (moved, _) = live
            .presence(&assumed.session, reach.clone(), &[])
            .expect("a document");
        assert_eq!(moved, first + 1);

        live.subscribe(&assumed.session, &a_loop("flight").id);
        let (again, _) = live
            .presence(&assumed.session, reach, &[])
            .expect("a document");
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
        live.presence(session, Vec::new(), &[])
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
                volumes: Vec::new(),
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
                volumes: Vec::new(),
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
                volumes: Vec::new(),
            })
            .expect("the seat to be free");
        let (first, _) = live
            .presence(&assumed.session, Vec::new(), &[])
            .expect("a document");

        live.the_client_says(&assumed.session, MediaPath::Connected);
        live.the_server_sees(&assumed.session, MediaPath::Connected);
        let (then, _) = live
            .presence(&assumed.session, Vec::new(), &[])
            .expect("a document");

        assert!(then > first, "the document moved and the version did not");

        // And it does not move for a reading that changes nothing.
        live.the_client_says(&assumed.session, MediaPath::Connected);
        let (again, _) = live
            .presence(&assumed.session, Vec::new(), &[])
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
                volumes: Vec::new(),
            })
            .expect("the seat to be free");
        let two = live
            .assume(Assuming {
                sign_in: two_sign_in,
                occupant: two_user,
                role: two_role,
                limit: None,
                subscribed_to: Vec::new(),
                volumes: Vec::new(),
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
                volumes: Vec::new(),
            })
            .expect("the seat to be free");
        live.ended_by_its_own_holder(&assumed.session);

        live.the_client_says(&assumed.session, MediaPath::Connected);
        live.the_server_sees(&assumed.session, MediaPath::Connected);

        assert!(live.presence(&assumed.session, Vec::new(), &[]).is_none());
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
        live.presence(session, within, &[])
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
        live.presence(session, within, &[])
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

        live.presence(&session, reach.clone(), &[]);
        live.arm(&session, &LoopId::presented("air-to-ground".to_owned()));
        live.subscribe(&session, &LoopId::presented("sim".to_owned()));

        let (_, presence) = live.presence(&session, reach, &[]).expect("a document");
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
        live.presence(&session, reach.clone(), &[]);

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

        live.presence(&session, emitting.clone(), &[]);
        live.arm(&session, &held_on);
        live.subscribe(&session, &held_on);

        // The cell goes to `none`, so the loop leaves the document altogether.
        live.presence(&session, Vec::new(), &[]);

        let (_, back) = live.presence(&session, emitting, &[]).expect("a document");
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
        live.presence(&session, vec![a_loop_to_emit_on("air-to-ground")], &[]);
        live.arm(&session, &LoopId::presented("air-to-ground".to_owned()));

        let (_, presence) = live
            .presence(&session, vec![a_loop("air-to-ground")], &[])
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

        live.presence(&talker, vec![a_loop_to_emit_on("air-to-ground")], &[]);
        live.presence(&listener, vec![a_loop("air-to-ground")], &[]);
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
        live.presence(&talker, vec![a_loop_to_emit_on("air-to-ground")], &[]);
        live.presence(&listener, vec![a_loop("air-to-ground")], &[]);
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
        live.presence(&talker, reach.clone(), &[]);
        live.presence(&listener, reach, &[]);

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
        live.presence(&talker, vec![a_loop_to_emit_on("air-to-ground")], &[]);
        live.presence(&listener, vec![a_loop("air-to-ground")], &[]);
        live.arm(&talker, &air_to_ground);
        live.subscribe(&listener, &air_to_ground);
        assert_eq!(heard_by(&live, &talker).len(), 1);

        // The listener's cell goes to `none`. The subscription stands and stops being heard.
        live.presence(&listener, Vec::new(), &[]);

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
        live.presence(&talker, vec![a_loop_to_emit_on("air-to-ground")], &[]);
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
        live.presence(&talker, reach.clone(), &[]);
        live.presence(&listener, reach, &[]);

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
        live.presence(&talker, vec![a_loop_to_emit_on("air-to-ground")], &[]);

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
            live.presence(session, emitting.clone(), &[]);
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
        live.presence(&blind, emitting.clone(), &[]);
        live.presence(&talker, emitting.clone(), &[]);
        live.arm(&blind, &air_to_ground);
        live.arm(&talker, &air_to_ground);
        live.the_client_keys(&talker);

        let (_, presence) = live.presence(&blind, emitting, &[]).expect("a document");

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
                .presence(&session, Vec::new(), &[])
                .expect("a document")
                .1
                .keyed
        );

        live.the_client_keys(&session);

        assert!(live.is_keyed(&session));
        assert!(
            live.presence(&session, Vec::new(), &[])
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
        let (before, _) = live
            .presence(&session, Vec::new(), &[])
            .expect("a document");

        live.the_client_keys(&session);
        let (after, _) = live
            .presence(&session, Vec::new(), &[])
            .expect("a document");

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

    /// A seat just taken has demonstrably been heard from: the act that created it arrived on
    /// the socket about to carry its heartbeats.
    #[tokio::test]
    async fn a_session_starts_confirmed() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;

        assert_eq!(live.connection_of(&session), Some(Connection::Confirmed));
    }

    /// **The three rungs, at the thresholds v1 §7 fixes**: 5 s unheard from is `unconfirmed`
    /// and 12 s is `disconnected`. The band between them is what makes withdrawing emission
    /// safe rather than fragile — a single threshold would mute the Flight Director
    /// mid-sentence for a VPN reroute.
    #[tokio::test]
    async fn the_ladder_climbs_at_five_seconds_and_at_twelve() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;

        for (unheard_for, rung) in [
            (Duration::from_secs(4), Connection::Confirmed),
            (Duration::from_secs(5), Connection::Unconfirmed),
            (Duration::from_secs(11), Connection::Unconfirmed),
            (Duration::from_secs(12), Connection::Disconnected),
            (Duration::from_secs(600), Connection::Disconnected),
        ] {
            live.unheard_from_for(&session, unheard_for);

            assert_eq!(
                live.connection_of(&session),
                Some(rung),
                "unheard from for {unheard_for:?}"
            );
        }
    }

    /// A heartbeat answered is the only thing that moves the ladder back down, and it moves
    /// it the whole way: the gap it is measured from starts again.
    #[tokio::test]
    async fn a_heartbeat_confirms_the_channel_again() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;
        live.unheard_from_for(&session, Duration::from_secs(30));

        live.the_client_is_there(&session);

        assert_eq!(live.connection_of(&session), Some(Connection::Confirmed));
    }

    /// **A closed socket is a fact rather than a silence**, so it does not wait out the
    /// ladder. A tab closed or a network reset is `disconnected` at once, and the rungs are
    /// left to the case nobody reported.
    #[tokio::test]
    async fn a_channel_known_to_have_gone_is_disconnected_at_once() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;

        live.the_channel_is_gone(&session);

        assert_eq!(live.connection_of(&session), Some(Connection::Disconnected));
        assert!(
            live.is_held_by(&session, &live_occupant(&live, &session)),
            "losing the channel ended the session, which is the reconnection window's call"
        );
    }

    /// **Emission still stands at `unconfirmed`** (ADR-0018). *We cannot confirm your
    /// transmission right now* is a materially different statement from *we know you are
    /// disconnected*, and cutting somebody off mid-word for a 500 ms blip is exactly the
    /// failure the band exists to prevent.
    #[tokio::test]
    async fn an_unconfirmed_talker_still_reaches_their_audience() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let (talker, listener) = a_talker_and_a_listener(&live, &store).await;

        live.unheard_from_for(&talker, Duration::from_secs(6));

        assert_eq!(
            heard_by(&live, &talker),
            vec![(listener.as_str().to_owned(), "air-to-ground".to_owned())],
            "an unconfirmed session lost its fan-out, which is the disconnect threshold's job"
        );
    }

    /// **The server closes the fan-out** (ADR-0018), and it does it independently of the
    /// client. Disabling push-to-talk in the client is not sufficient: the situation that
    /// motivates the rule is the one where the client may be wedged, and a wedged client's
    /// audio is still arriving at a perfectly healthy media transport.
    #[tokio::test]
    async fn a_disconnected_talker_reaches_nobody() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let (talker, _listener) = a_talker_and_a_listener(&live, &store).await;

        live.unheard_from_for(&talker, Duration::from_secs(12));

        assert!(
            heard_by(&live, &talker).is_empty(),
            "a session nobody could be told about was still routed to its loops"
        );
    }

    /// The arms stand through it. Nothing has taken the rung away, the operator chose them,
    /// and they are what the console comes back to — so the fan-out closes and reopens rather
    /// than being rebuilt by hand.
    #[tokio::test]
    async fn the_fan_out_reopens_when_the_channel_does() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let (talker, listener) = a_talker_and_a_listener(&live, &store).await;
        live.unheard_from_for(&talker, Duration::from_secs(12));
        assert!(heard_by(&live, &talker).is_empty());

        live.the_client_is_there(&talker);

        assert_eq!(
            heard_by(&live, &talker),
            vec![(listener.as_str().to_owned(), "air-to-ground".to_owned())]
        );
    }

    /// **Every talking indicator anyone sees is a server broadcast** (ADR-0008), so a session
    /// nobody can be told about is not shown talking on anybody's console — which is the
    /// whole argument for taking its emission path away rather than a second rule.
    #[tokio::test]
    async fn a_disconnected_talker_is_not_shown_talking() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let (talker, listener) = a_talker_and_a_listener(&live, &store).await;
        live.the_client_keys(&talker);
        assert_eq!(
            talking(&live, &listener, vec![a_loop("air-to-ground")]),
            vec!["air-to-ground".to_owned()]
        );

        live.unheard_from_for(&talker, Duration::from_secs(12));

        assert!(
            talking(&live, &listener, vec![a_loop("air-to-ground")]).is_empty(),
            "a console showed a transmission nobody was receiving"
        );
    }

    /// The talker's own lamp goes out with it, and for the same reason: the lamp is the
    /// server's answer about whether a transmission is happening, and past the threshold none
    /// is (ADR-0008).
    #[tokio::test]
    async fn a_disconnected_session_is_not_keyed_in_its_own_document() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let (talker, _listener) = a_talker_and_a_listener(&live, &store).await;
        live.the_client_keys(&talker);

        live.unheard_from_for(&talker, Duration::from_secs(12));

        let (_, presence) = live
            .presence(&talker, vec![a_loop_to_emit_on("air-to-ground")], &[])
            .expect("a document");
        assert!(!presence.keyed);
    }

    /// **The server's own reading rides in the document** ([ADR-0018]), for the half of the
    /// failure where it can still be heard: a console whose answers are being lost while the
    /// server's heartbeats still arrive has no way of its own to know its fan-out has closed.
    #[tokio::test]
    async fn the_document_carries_the_server_s_reading_of_the_channel() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;
        let reach = vec![a_loop_to_emit_on("air-to-ground")];

        for (unheard_for, rung) in [
            (Duration::ZERO, Connection::Confirmed),
            (Duration::from_secs(5), Connection::Unconfirmed),
            (Duration::from_secs(12), Connection::Disconnected),
        ] {
            live.unheard_from_for(&session, unheard_for);

            let (_, presence) = live
                .presence(&session, reach.clone(), &[])
                .expect("a document");
            assert_eq!(
                presence.connection, rung,
                "unheard from for {unheard_for:?}"
            );
        }
    }

    /// The rung moves the version, because the console renders it. The **age** is deliberately
    /// not in the document for the opposite reason: a number that moved five times a second
    /// would make *is this the same state* unanswerable, which is the one question versioning
    /// is for.
    #[tokio::test]
    async fn the_version_moves_when_the_channel_s_rung_does_and_not_with_its_age() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;
        let reach = vec![a_loop_to_emit_on("air-to-ground")];
        let (first, _) = live
            .presence(&session, reach.clone(), &[])
            .expect("a document");

        live.unheard_from_for(&session, Duration::from_secs(3));
        let (aged, _) = live
            .presence(&session, reach.clone(), &[])
            .expect("a document");
        assert_eq!(aged, first, "the version moved for an age nothing renders");

        live.unheard_from_for(&session, Duration::from_secs(6));
        let (moved, _) = live.presence(&session, reach, &[]).expect("a document");
        assert_eq!(moved, first + 1);
    }

    // ---- Mute and per-loop volume (#44) -------------------------------------------------

    /// The one loop's standing on one session's console, as its own document has it.
    fn standing_of(live: &StateAuthority, session: &SessionId, within: Vec<InReach>) -> Standing {
        live.presence(session, within, &[])
            .expect("a document")
            .1
            .loops
            .into_iter()
            .next()
            .expect("one loop in reach")
    }

    /// **Mute is not an unsubscribe** (v1 §5). The subscription stands, which is what keeps
    /// the talking indicator, loop health and the priority mark arriving on a muted loop.
    #[tokio::test]
    async fn muting_a_loop_leaves_it_monitored_and_says_it_is_muted() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;
        let air_to_ground = LoopId::presented("air-to-ground".to_owned());
        live.subscribe(&session, &air_to_ground);

        assert!(live.mute(&session, &air_to_ground));

        let standing = standing_of(&live, &session, vec![a_loop("air-to-ground")]);
        assert!(standing.subscribed, "muting a loop took it off the console");
        assert!(
            standing.muted,
            "the document does not say the loop is muted"
        );

        assert!(live.unmute(&session, &air_to_ground));
        assert!(!standing_of(&live, &session, vec![a_loop("air-to-ground")]).muted);
    }

    /// **Mute silences a loop in the muter's own ears**, so the fan-out stops carrying the
    /// talker to them on it. That is also why priority cannot defeat a mute (ADR-0045): there
    /// is no carriage for a gain to be raised on.
    #[tokio::test]
    async fn a_muted_loop_carries_no_talker_to_the_operator_who_muted_it() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let (talker, listener) = a_talker_and_a_listener(&live, &store).await;
        let air_to_ground = LoopId::presented("air-to-ground".to_owned());

        live.mute(&listener, &air_to_ground);
        assert!(
            heard_by(&live, &talker).is_empty(),
            "a muted loop still carried"
        );

        live.unmute(&listener, &air_to_ground);
        assert_eq!(
            heard_by(&live, &talker),
            [(listener.as_str().to_owned(), "air-to-ground".to_owned())],
            "unmuting did not put the talker back"
        );
    }

    /// **It affects nobody else.** A mute is a personalisation and not a permission: every
    /// other listener on the loop goes on hearing, and the talker is still carried to them.
    #[tokio::test]
    async fn a_mute_changes_nothing_for_anybody_else_on_the_loop() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let (talker, listener) = a_talker_and_a_listener(&live, &store).await;
        let other = a_session(&live, &store, "gnc").await;
        let air_to_ground = LoopId::presented("air-to-ground".to_owned());
        live.presence(&other, vec![a_loop("air-to-ground")], &[]);
        live.subscribe(&other, &air_to_ground);

        live.mute(&listener, &air_to_ground);

        assert_eq!(
            heard_by(&live, &talker),
            [(other.as_str().to_owned(), "air-to-ground".to_owned())]
        );
        assert!(!standing_of(&live, &other, vec![a_loop("air-to-ground")]).muted);
    }

    /// A talker reaching the operator on two loops is still heard when one of them is muted:
    /// **the mute silences the loop, not the voice**, and the other loop is one the operator
    /// kept up.
    #[tokio::test]
    async fn a_talker_on_a_muted_loop_and_an_unmuted_one_is_heard_on_the_unmuted_one() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let talker = a_session(&live, &store, "flight").await;
        let listener = a_session(&live, &store, "capcom").await;
        let reach = vec![a_loop_to_emit_on("air-to-ground"), a_loop_to_emit_on("sim")];
        live.presence(&talker, reach.clone(), &[]);
        live.presence(&listener, reach, &[]);
        for held_on in ["air-to-ground", "sim"] {
            live.arm(&talker, &LoopId::presented(held_on.to_owned()));
            live.subscribe(&listener, &LoopId::presented(held_on.to_owned()));
        }

        live.mute(&listener, &LoopId::presented("sim".to_owned()));

        assert_eq!(
            heard_by(&live, &talker),
            [(listener.as_str().to_owned(), "air-to-ground".to_owned())]
        );
    }

    /// The subscription stands, so **the talking indicator keeps arriving** on a muted loop:
    /// an operator asked for silence, not blindness (ADR-0059).
    #[tokio::test]
    async fn a_muted_loop_still_says_somebody_is_talking_on_it() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let (talker, listener) = a_talker_and_a_listener(&live, &store).await;
        live.the_client_keys(&talker);

        live.mute(&listener, &LoopId::presented("air-to-ground".to_owned()));

        assert_eq!(
            talking(&live, &listener, vec![a_loop("air-to-ground")]),
            ["air-to-ground"]
        );
    }

    /// **A mute presupposes a subscription** (ADR-0049). There is nothing to silence on a
    /// loop nobody is hearing, so muting one leaves nothing behind for a later subscribe to
    /// find.
    #[tokio::test]
    async fn muting_a_loop_nobody_is_monitoring_leaves_nothing_behind() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;
        let air_to_ground = LoopId::presented("air-to-ground".to_owned());

        live.mute(&session, &air_to_ground);
        live.subscribe(&session, &air_to_ground);

        assert!(
            !standing_of(&live, &session, vec![a_loop("air-to-ground")]).muted,
            "a loop taken up arrived muted"
        );
    }

    /// **A mute is dropped with its subscription** (ADR-0049), so taking a loop back up later
    /// is taking it up audible.
    #[tokio::test]
    async fn dropping_a_loop_drops_its_mute() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;
        let air_to_ground = LoopId::presented("air-to-ground".to_owned());
        live.subscribe(&session, &air_to_ground);
        live.mute(&session, &air_to_ground);

        live.unsubscribe(&session, &air_to_ground);
        live.subscribe(&session, &air_to_ground);

        assert!(!standing_of(&live, &session, vec![a_loop("air-to-ground")]).muted);
    }

    /// **A mute is never remembered** (ADR-0050): a forgotten one would silence a loop the
    /// moment its owner assumed the role again, and drop every loop they staff to `away` for
    /// the whole room before they had looked at anything. The subscription comes back; the
    /// mute does not.
    #[tokio::test]
    async fn a_mute_ends_with_the_session_and_the_next_assume_hears_the_loop() {
        let (_directory, store) = a_temporary_store().await;
        let (sign_in, user, role) = a_seat(&store, "flight", "Flight Director").await;
        let live = StateAuthority::empty();
        let air_to_ground = LoopId::presented("air-to-ground".to_owned());
        let remembered = || Assuming {
            subscribed_to: vec![air_to_ground.clone()],
            ..taking(&sign_in, &user, &role, Some(1))
        };
        let assumed = live.assume(remembered()).expect("the seat to be free");
        live.mute(&assumed.session, &air_to_ground);
        live.ended_by_its_own_holder(&assumed.session)
            .expect("the session to end");

        let again = live.assume(remembered()).expect("the seat to be free");

        let standing = standing_of(&live, &again.session, vec![a_loop("air-to-ground")]);
        assert!(
            standing.subscribed,
            "the remembered subscription did not come back"
        );
        assert!(!standing.muted, "a mute outlived the session that set it");
    }

    /// **A mute does not auto-expire** (v1 §5): an unexpected un-mute mid-incident is its own
    /// hazard. Nothing the clock or the channel does takes one away — a session that goes
    /// `disconnected` and comes back comes back muted.
    #[tokio::test]
    async fn a_mute_survives_the_channel_going_and_coming_back() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;
        let air_to_ground = LoopId::presented("air-to-ground".to_owned());
        live.subscribe(&session, &air_to_ground);
        live.mute(&session, &air_to_ground);

        live.unheard_from_for(&session, Duration::from_secs(3600));
        live.the_channel_is_gone(&session);
        live.the_client_is_there(&session);

        assert!(standing_of(&live, &session, vec![a_loop("air-to-ground")]).muted);
    }

    #[tokio::test]
    async fn muting_on_a_session_nobody_holds_changes_nothing() {
        let live = StateAuthority::empty();
        let nobody = SessionId::presented("nothing".to_owned());

        assert!(!live.mute(&nobody, &a_loop("flight").id));
        assert!(!live.unmute(&nobody, &a_loop("flight").id));
        assert!(!live.set_the_volume(&nobody, &a_loop("flight").id, Volume::UNITY));
    }

    /// **Every loop starts at unity** (v1 §10).
    #[tokio::test]
    async fn every_loop_starts_at_unity() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;

        assert_eq!(
            standing_of(&live, &session, vec![a_loop("air-to-ground")]).volume,
            Volume::UNITY
        );
    }

    #[tokio::test]
    async fn a_volume_set_is_the_volume_the_document_carries_and_moves_its_version() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;
        let reach = vec![a_loop("air-to-ground")];
        let (first, _) = live
            .presence(&session, reach.clone(), &[])
            .expect("a document");
        let turned_down = Volume::presented(40).expect("a volume");

        assert!(live.set_the_volume(&session, &a_loop("air-to-ground").id, turned_down));

        let (moved, presence) = live.presence(&session, reach, &[]).expect("a document");
        assert_eq!(presence.loops[0].volume, turned_down);
        assert_eq!(moved, first + 1);
    }

    /// Volume persists and is handed in at assume like the subscription set is, which is what
    /// makes a restart cost an assume rather than a console rebuilt by hand (ADR-0050).
    #[tokio::test]
    async fn assuming_restores_the_volumes_the_pair_last_had() {
        let (_directory, store) = a_temporary_store().await;
        let (sign_in, user, role) = a_seat(&store, "flight", "Flight Director").await;
        let live = StateAuthority::empty();
        let turned_down = Volume::presented(25).expect("a volume");
        let assumed = live
            .assume(Assuming {
                volumes: vec![(a_loop("sim").id, turned_down)],
                ..taking(&sign_in, &user, &role, Some(1))
            })
            .expect("the seat to be free");

        let (_, presence) = live
            .presence(
                &assumed.session,
                vec![a_loop("air-to-ground"), a_loop("sim")],
                &[],
            )
            .expect("a document");
        assert_eq!(presence.loops[0].volume, Volume::UNITY);
        assert_eq!(presence.loops[1].volume, turned_down);
    }

    /// **The grid overrules personalisation silently and keeps it inert** (ADR-0051). A loop
    /// that leaves reach and comes back comes back at the level it left at.
    #[tokio::test]
    async fn a_volume_outside_reach_is_kept_and_comes_back_with_it() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;
        let turned_down = Volume::presented(10).expect("a volume");
        live.set_the_volume(&session, &a_loop("sim").id, turned_down);

        live.presence(&session, vec![a_loop("air-to-ground")], &[]);

        assert_eq!(
            standing_of(&live, &session, vec![a_loop("sim")]).volume,
            turned_down
        );
    }

    /// **Volume is not a route.** A loop turned all the way down is still a loop the operator
    /// is hearing as far as the fan-out goes, because loudest-wins is settled at the client
    /// over every loop a talker reaches them on — and a talker also on a loop they kept up is
    /// one they have said they want to hear (v1 §5). Silencing a loop outright is what mute is
    /// for.
    #[tokio::test]
    async fn a_loop_turned_down_to_nothing_still_carries_the_talker() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let (talker, listener) = a_talker_and_a_listener(&live, &store).await;

        live.set_the_volume(
            &listener,
            &LoopId::presented("air-to-ground".to_owned()),
            Volume::presented(0).expect("a volume"),
        );

        assert_eq!(
            heard_by(&live, &talker),
            [(listener.as_str().to_owned(), "air-to-ground".to_owned())]
        );
    }

    /// A talker armed and a listener monitoring the same loop, which is the smallest thing
    /// with a fan-out in it.
    async fn a_talker_and_a_listener(
        live: &StateAuthority,
        store: &Store,
    ) -> (SessionId, SessionId) {
        let talker = a_session(live, store, "flight").await;
        let listener = a_session(live, store, "capcom").await;
        let air_to_ground = LoopId::presented("air-to-ground".to_owned());

        live.presence(&talker, vec![a_loop_to_emit_on("air-to-ground")], &[]);
        live.presence(&listener, vec![a_loop("air-to-ground")], &[]);
        live.arm(&talker, &air_to_ground);
        live.subscribe(&listener, &air_to_ground);

        (talker, listener)
    }

    // ---- Priority (#45) -----------------------------------------------------------------

    /// The loops one session's document marks as carrying a priority transmission.
    fn marked(live: &StateAuthority, session: &SessionId, within: Vec<InReach>) -> Vec<String> {
        live.presence(session, within, &[])
            .expect("a document")
            .1
            .loops
            .into_iter()
            .filter(|standing| standing.priority)
            .map(|standing| standing.held_on.name)
            .collect()
    }

    /// A talker armed on two loops, and a listener monitoring both.
    async fn a_talker_on_two_loops(
        live: &StateAuthority,
        store: &Store,
    ) -> (SessionId, SessionId, Vec<InReach>) {
        let talker = a_session(live, store, "flight").await;
        let listener = a_session(live, store, "capcom").await;
        let reach = vec![a_loop_to_emit_on("air-to-ground"), a_loop_to_emit_on("sim")];
        live.presence(&talker, reach.clone(), &[]);
        live.presence(&listener, reach.clone(), &[]);
        for held_on in ["air-to-ground", "sim"] {
            live.arm(&talker, &LoopId::presented(held_on.to_owned()));
            live.subscribe(&listener, &LoopId::presented(held_on.to_owned()));
        }

        (talker, listener, reach)
    }

    /// **Priority applies to the whole arm set** (ADR-0045). One stream is fanned out at the
    /// server, so a priority transmission is marked on every loop it lands on, and there is no
    /// way to be priority on one armed loop and ordinary on another.
    #[tokio::test]
    async fn a_priority_transmission_is_marked_on_every_loop_it_lands_on() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let (talker, listener, reach) = a_talker_on_two_loops(&live, &store).await;

        live.the_client_keys(&talker);
        assert!(
            marked(&live, &listener, reach.clone()).is_empty(),
            "an ordinary transmission was marked"
        );

        assert!(live.the_client_keys_priority(&talker));
        assert_eq!(
            marked(&live, &listener, reach.clone()),
            ["air-to-ground", "sim"]
        );
        assert_eq!(
            talking(&live, &listener, reach.clone()),
            ["air-to-ground", "sim"]
        );

        live.the_client_unkeys_priority(&talker);
        assert!(marked(&live, &listener, reach.clone()).is_empty());
        assert_eq!(
            talking(&live, &listener, reach),
            ["air-to-ground", "sim"],
            "letting go of priority under a key ended the transmission"
        );
    }

    /// **Marked wherever it lands** (ADR-0059): on a loop the receiver has muted, where there is
    /// no audio at all, and on one they are not monitoring. The mark is a declaration that
    /// somebody called this urgent, not an explanation of a gain change.
    #[tokio::test]
    async fn the_mark_reaches_a_muted_loop_and_an_unmonitored_one() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let (talker, listener) = a_talker_and_a_listener(&live, &store).await;
        let unmonitored = a_session(&live, &store, "gnc").await;
        live.presence(&unmonitored, vec![a_loop("air-to-ground")], &[]);
        live.mute(&listener, &LoopId::presented("air-to-ground".to_owned()));

        live.the_client_keys(&talker);
        live.the_client_keys_priority(&talker);

        assert_eq!(
            marked(&live, &listener, vec![a_loop("air-to-ground")]),
            ["air-to-ground"],
            "a mute hid the mark"
        );
        assert_eq!(
            marked(&live, &unmonitored, vec![a_loop("air-to-ground")]),
            ["air-to-ground"],
            "the mark reached only the loops somebody was hearing"
        );
    }

    /// **Priority governs gain and never who receives** (ADR-0045). It defeats no mute,
    /// compels no subscription and lowers no other talker, so the fan-out is exactly what it
    /// was: nothing is added, nothing is taken away, and nothing moves.
    #[tokio::test]
    async fn priority_changes_nobody_s_route() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let (talker, listener) = a_talker_and_a_listener(&live, &store).await;
        let other = a_session(&live, &store, "gnc").await;
        let unsubscribed = a_session(&live, &store, "eecom").await;
        let air_to_ground = LoopId::presented("air-to-ground".to_owned());
        live.presence(&other, vec![a_loop_to_emit_on("air-to-ground")], &[]);
        live.presence(&unsubscribed, vec![a_loop("air-to-ground")], &[]);
        live.arm(&other, &air_to_ground);
        live.mute(&listener, &air_to_ground);
        live.the_client_keys(&other);
        live.the_routing_if_it_moved();

        live.the_client_keys(&talker);
        live.the_client_keys_priority(&talker);

        assert_eq!(
            live.the_routing_if_it_moved(),
            None,
            "keying priority moved somebody's route"
        );
        assert!(
            heard_by(&live, &talker).is_empty(),
            "priority defeated a mute or compelled a subscription"
        );
    }

    /// **The level model, on the server's side** (ADR-0046): `is-priority` is the priority
    /// level of a transmission, so a priority level with nothing keyed marks nothing — there is
    /// no transmission for it to be an attribute of.
    #[tokio::test]
    async fn a_priority_level_under_no_key_marks_nothing() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let (talker, listener) = a_talker_and_a_listener(&live, &store).await;

        live.the_client_keys_priority(&talker);

        assert!(marked(&live, &listener, vec![a_loop("air-to-ground")]).is_empty());
        assert!(
            !live
                .presence(&talker, vec![a_loop_to_emit_on("air-to-ground")], &[])
                .expect("a document")
                .1
                .priority
        );
    }

    /// **Cut beats priority** (ADR-0045). A transmission whose fan-out is closed is not marked
    /// anywhere, because the mark is an attribute of a transmission that is landing. The talker
    /// with no signalling channel is the one whose fan-out VoxLoop closes today, by the machinery
    /// Cut will use.
    #[tokio::test]
    async fn a_priority_talker_whose_fan_out_is_closed_is_marked_nowhere() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let (talker, listener) = a_talker_and_a_listener(&live, &store).await;
        live.the_client_keys(&talker);
        live.the_client_keys_priority(&talker);

        live.unheard_from_for(&talker, Duration::from_secs(13));

        assert!(marked(&live, &listener, vec![a_loop("air-to-ground")]).is_empty());
        assert!(heard_by(&live, &talker).is_empty());
    }

    /// The talker's own lamp says their transmission is at priority, and it says so from the
    /// server's answer — an elevated latch shows as elevated with no new surface (ADR-0046).
    #[tokio::test]
    async fn the_lamp_says_when_a_transmission_is_at_priority() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;
        live.the_client_keys(&session);
        let (before, ordinary) = live
            .presence(&session, Vec::new(), &[])
            .expect("a document");
        assert!(!ordinary.priority);

        live.the_client_keys_priority(&session);
        let (after, elevated) = live
            .presence(&session, Vec::new(), &[])
            .expect("a document");

        assert!(elevated.priority);
        assert_eq!(
            after,
            before + 1,
            "keying priority did not move the document"
        );
    }

    /// **Every press is handed back to be audited, with the armed set as it stood at the
    /// press** (v1 §12). A set that moved during the press is still recorded as it was when
    /// the key went down, because that is the set the priority was keyed over.
    #[tokio::test]
    async fn a_press_is_handed_back_when_it_ends_with_the_arm_set_it_was_keyed_over() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let (talker, _listener, _reach) = a_talker_on_two_loops(&live, &store).await;
        let occupant = live_occupant(&live, &talker);

        live.the_client_keys(&talker);
        live.the_client_keys_priority(&talker);
        live.disarm(&talker, &LoopId::presented("sim".to_owned()));
        let pressed = live
            .the_client_unkeys_priority(&talker)
            .expect("the press to be handed back");

        assert_eq!(pressed.occupant, occupant);
        assert_eq!(
            Some(pressed.role),
            live.the_role_of(&talker),
            "the press was not attributed to the role it was keyed under"
        );
        assert_eq!(pressed.armed_on, ["air-to-ground", "sim"]);
        assert!(pressed.lasted < Duration::from_secs(5));
    }

    /// **No minimum duration** (ADR-0046). A press that lasted no time at all is still a press,
    /// and a second key-down while one is held is the same press rather than a new one.
    #[tokio::test]
    async fn every_press_is_one_press_however_short_and_however_often_it_is_said() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;

        live.the_client_keys_priority(&session);
        live.the_client_keys_priority(&session);
        assert!(live.the_client_unkeys_priority(&session).is_some());
        assert!(
            live.the_client_unkeys_priority(&session).is_none(),
            "one press was handed back twice"
        );
    }

    /// A press held when the session ends ends with it, and the ending hands it back: the
    /// press happened, and relinquishing under it is not a way to keep it out of the log.
    #[tokio::test]
    async fn a_press_held_when_the_session_ends_is_handed_back_with_the_ending() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;
        live.the_client_keys_priority(&session);

        let ended = live
            .ended_by_its_own_holder(&session)
            .expect("the session to end");

        assert!(
            ended.pressed.is_some(),
            "a press went unrecorded with its session"
        );
    }

    /// **A priority key held across an outage is suppressed until released** (ADR-0043), so a
    /// socket that goes takes the press with it. Nothing is left standing to raise anybody's
    /// volume when a new socket comes back.
    #[tokio::test]
    async fn a_press_ends_when_the_channel_goes() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;
        live.the_client_keys(&session);
        live.the_client_keys_priority(&session);

        assert!(live.the_channel_is_gone(&session).is_some());

        live.the_client_is_there(&session);
        assert!(
            !live
                .presence(&session, Vec::new(), &[])
                .expect("a document")
                .1
                .priority,
            "a press outlived the channel it was keyed on"
        );
        assert!(live.the_client_unkeys_priority(&session).is_none());
    }

    // ---- The loop beacon and loop health (#46) --------------------------------------------

    /// This session's health on the one loop in reach, as its own document has it.
    fn health_of(live: &StateAuthority, session: &SessionId, name: &str) -> Option<LoopHealth> {
        standing_of(live, session, vec![a_loop(name)]).health
    }

    /// The client's count of one loop's beacon, as it would report it.
    fn counted(name: &str, packets: u64) -> Vec<(LoopId, u64)> {
        vec![(LoopId::presented(name.to_owned()), packets)]
    }

    /// Longer than a beacon may go unheard before the loop is lost.
    const PAST_THE_WINDOW: Duration = Duration::from_secs(16);

    /// **A loop just taken up has not yet proved it reaches anybody**, and it says so rather
    /// than guessing either way. The first beacon is at most one interval off.
    #[tokio::test]
    async fn a_loop_just_taken_up_is_being_checked() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;
        live.subscribe(&session, &LoopId::presented("flight".to_owned()));

        assert_eq!(
            health_of(&live, &session, "flight"),
            Some(LoopHealth::Checking)
        );
    }

    /// **Loop health is measured from the beacon's arrival** (ADR-0017): a count that moves is
    /// a packet that crossed the same transport, router and fan-out that speech would.
    #[tokio::test]
    async fn a_beacon_counted_is_a_loop_being_received() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;
        live.subscribe(&session, &LoopId::presented("flight".to_owned()));

        live.the_client_counted(&session, &counted("flight", 1));

        assert_eq!(
            health_of(&live, &session, "flight"),
            Some(LoopHealth::Receiving)
        );
    }

    /// **A beacon that stops arriving is a loop this session is deaf to**, and a count said
    /// again unchanged is not an arrival: it is the client reporting that nothing came.
    #[tokio::test]
    async fn a_beacon_that_stops_arriving_is_a_loop_not_being_received() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;
        live.subscribe(&session, &LoopId::presented("flight".to_owned()));
        live.the_client_counted(&session, &counted("flight", 3));

        live.the_beacons_went_unheard_for(&session, PAST_THE_WINDOW);
        live.the_client_is_there(&session);
        live.the_client_counted(&session, &counted("flight", 3));

        assert_eq!(
            health_of(&live, &session, "flight"),
            Some(LoopHealth::NotReceiving)
        );
    }

    /// **A wedged client fails safe** (ADR-0017). It reports nothing, so nothing arrives, and
    /// the loop is read as not received rather than left at *checking* for ever.
    #[tokio::test]
    async fn a_client_that_never_reports_is_not_receiving_the_loop() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;
        live.subscribe(&session, &LoopId::presented("flight".to_owned()));

        live.the_beacons_went_unheard_for(&session, PAST_THE_WINDOW);

        assert_eq!(
            health_of(&live, &session, "flight"),
            Some(LoopHealth::NotReceiving)
        );
    }

    /// **Loop health is per (session, loop)**, so two subscribers may correctly disagree about
    /// the same loop, and both readings are right.
    #[tokio::test]
    async fn two_subscribers_may_disagree_about_the_same_loop() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let receiving = a_session(&live, &store, "flight").await;
        let deaf = a_session(&live, &store, "capcom").await;
        let flight = LoopId::presented("flight".to_owned());
        live.subscribe(&receiving, &flight);
        live.subscribe(&deaf, &flight);
        live.the_beacons_went_unheard_for(&receiving, PAST_THE_WINDOW);
        live.the_beacons_went_unheard_for(&deaf, PAST_THE_WINDOW);

        live.the_client_counted(&receiving, &counted("flight", 1));

        assert_eq!(
            health_of(&live, &receiving, "flight"),
            Some(LoopHealth::Receiving)
        );
        assert_eq!(
            health_of(&live, &deaf, "flight"),
            Some(LoopHealth::NotReceiving)
        );
    }

    /// A loop nobody here is monitoring has no health on this console: its beacon is not
    /// consumed, so there is nothing to measure. And a count for it is not taken as one.
    #[tokio::test]
    async fn a_loop_not_monitored_has_no_health_and_a_count_for_it_is_ignored() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;

        live.the_client_counted(&session, &counted("flight", 5));

        assert_eq!(health_of(&live, &session, "flight"), None);
        live.subscribe(&session, &LoopId::presented("flight".to_owned()));
        assert_eq!(
            health_of(&live, &session, "flight"),
            Some(LoopHealth::Checking),
            "a count from before the loop was taken up was read as an arrival"
        );
    }

    /// **A mute is not an unsubscribe** (v1 §5): the beacon keeps arriving on a muted loop,
    /// and so does its health.
    #[tokio::test]
    async fn a_muted_loop_keeps_its_health() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;
        let flight = LoopId::presented("flight".to_owned());
        live.subscribe(&session, &flight);
        live.mute(&session, &flight);

        live.the_client_counted(&session, &counted("flight", 1));

        assert_eq!(
            health_of(&live, &session, "flight"),
            Some(LoopHealth::Receiving)
        );
    }

    /// **Every subscriber consumes the beacon**, muted or not, and only within reach. A loop
    /// kept in the set but out of reach is inert (ADR-0051), beacon and all.
    #[tokio::test]
    async fn every_subscriber_in_reach_counts_the_beacon_of_each_loop_it_monitors() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;
        let flight = LoopId::presented("flight".to_owned());
        let sim = LoopId::presented("sim".to_owned());
        live.subscribe(&session, &flight);
        live.subscribe(&session, &sim);
        live.mute(&session, &flight);
        // Reach is recorded when a document is projected, and `sim` is not in it.
        live.presence(&session, vec![a_loop("flight")], &[]);

        let counting = live.the_beacons_if_they_moved().expect("an answer");

        assert_eq!(
            counting,
            vec![WhoCounts {
                listener: session.clone(),
                on: vec![flight.clone()]
            }]
        );
        assert!(
            live.the_beacons_if_they_moved().is_none(),
            "an answer that had not moved was handed down again"
        );

        live.unsubscribe(&session, &flight);
        assert_eq!(
            live.the_beacons_if_they_moved(),
            Some(vec![WhoCounts {
                listener: session,
                on: Vec::new()
            }])
        );
    }

    /// **Beacon loss is suppressed while connection state already explains the silence**, or
    /// one failure arrives as two competing reasons. The counts ride the signalling channel,
    /// so a channel in trouble stops them as a side effect.
    #[tokio::test]
    async fn beacon_loss_is_suppressed_while_the_connection_explains_it() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;
        let flight = LoopId::presented("flight".to_owned());
        live.subscribe(&session, &flight);
        live.presence(&session, vec![a_loop("flight")], &[]);
        live.the_beacons_went_unheard_for(&session, PAST_THE_WINDOW);

        live.unheard_from_for(&session, Duration::from_secs(6));
        assert_eq!(
            health_of(&live, &session, "flight"),
            None,
            "an unconfirmed channel was shown as a loop not received"
        );
        assert_eq!(
            live.why_not_hearing(&session, &flight),
            None,
            "an unconfirmed channel took the loop away from somebody who may be hearing it"
        );

        live.unheard_from_for(&session, Duration::from_secs(13));
        assert_eq!(
            live.why_not_hearing(&session, &flight),
            Some(NotHearing::Unreachable),
            "a lost channel was reported as beacon loss as well as, or instead of, itself"
        );
    }

    /// **Coming back starts the measurement again.** The counts stopped because the channel
    /// did, and a beacon last counted before the gap says nothing about now either way.
    #[tokio::test]
    async fn a_channel_that_comes_back_checks_the_loop_again() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;
        live.subscribe(&session, &LoopId::presented("flight".to_owned()));
        live.the_client_counted(&session, &counted("flight", 1));
        live.the_beacons_went_unheard_for(&session, PAST_THE_WINDOW);
        live.unheard_from_for(&session, PAST_THE_WINDOW);

        live.the_client_is_there(&session);

        assert_eq!(
            health_of(&live, &session, "flight"),
            Some(LoopHealth::Checking)
        );
    }

    /// **Beacon loss drops the loop to `away` for staffing purposes** (ADR-0017), which is
    /// what makes `staffed` mean *demonstrably receiving*. Checking is not loss: it is a
    /// measurement not yet taken, and it becomes loss if the window runs out on it.
    ///
    /// Within one occupant the reason reported is the one furthest upstream (v1 §8), which is
    /// how the suppression above generalises: not subscribed before not receiving it, and not
    /// receiving it before muted.
    #[tokio::test]
    async fn beacon_loss_is_a_reason_an_occupant_is_not_hearing_a_loop() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;
        let flight = LoopId::presented("flight".to_owned());
        live.presence(&session, vec![a_loop("flight")], &[]);
        assert_eq!(
            live.why_not_hearing(&session, &flight),
            Some(NotHearing::NotSubscribed)
        );

        live.subscribe(&session, &flight);
        assert_eq!(live.why_not_hearing(&session, &flight), None);

        live.mute(&session, &flight);
        assert_eq!(
            live.why_not_hearing(&session, &flight),
            Some(NotHearing::Muted)
        );

        live.the_beacons_went_unheard_for(&session, PAST_THE_WINDOW);
        live.the_client_is_there(&session);
        assert_eq!(
            live.why_not_hearing(&session, &flight),
            Some(NotHearing::NotReceiving)
        );
    }

    /// Taking a loop up again starts its count again: the client builds a fresh carriage for
    /// it, which counts from nothing.
    #[tokio::test]
    async fn a_loop_taken_up_again_is_checked_afresh() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;
        let flight = LoopId::presented("flight".to_owned());
        live.subscribe(&session, &flight);
        live.the_client_counted(&session, &counted("flight", 9));

        live.unsubscribe(&session, &flight);
        live.subscribe(&session, &flight);
        assert_eq!(
            health_of(&live, &session, "flight"),
            Some(LoopHealth::Checking)
        );

        live.the_client_counted(&session, &counted("flight", 1));
        assert_eq!(
            health_of(&live, &session, "flight"),
            Some(LoopHealth::Receiving)
        );
    }

    // ---- Off console (#47) ----------------------------------------------------------------

    /// What one session's own document says it has claimed about itself.
    fn asserted_by(live: &StateAuthority, session: &SessionId) -> Option<Asserted> {
        live.presence(session, Vec::new(), &[])
            .expect("a document")
            .1
            .off_console
    }

    /// **The claim and the age of its evidence arrive together** (ADR-0016, v1 §6). There is no
    /// document in which one of them is present and the other is not, because they are one
    /// value.
    #[tokio::test]
    async fn an_assertion_is_shown_with_how_long_ago_its_claimant_was_last_active() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;

        assert_eq!(
            asserted_by(&live, &session),
            None,
            "a seat just taken claims something"
        );

        live.last_active_was(&session, Duration::from_secs(14 * 60));
        assert!(live.off_console(&session));

        let asserted = asserted_by(&live, &session).expect("the assertion");
        assert_eq!(asserted.last_active.as_secs(), 14 * 60);
    }

    /// **The version moves when the age does and not five times a second** ([ADR-0019]). The
    /// age is part of the state, so the document does move while an assertion stands — once a
    /// second, which is the resolution the age is carried at, rather than on every tick with
    /// nothing to show for it.
    #[tokio::test]
    async fn the_version_moves_with_the_age_once_a_second_and_not_every_tick() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;
        live.off_console(&session);

        live.last_active_was(&session, Duration::from_secs(5));
        let (first, _) = live
            .presence(&session, Vec::new(), &[])
            .expect("a document");
        live.last_active_was(&session, Duration::from_millis(5_400));
        let (within_the_same_second, _) = live
            .presence(&session, Vec::new(), &[])
            .expect("a document");
        live.last_active_was(&session, Duration::from_secs(6));
        let (a_second_later, _) = live
            .presence(&session, Vec::new(), &[])
            .expect("a document");

        assert_eq!(
            within_the_same_second, first,
            "the version moved for an age the document does not carry"
        );
        assert!(
            a_second_later > first,
            "the age moved and the version did not"
        );
    }

    /// **A stale assertion is still shown, with its age** (v1 §6). Nothing expires it and
    /// nothing resolves it: the ambiguity of somebody who walked away is made visible and the
    /// judgement is left with the human, which is a deliberate refusal to be helpful.
    #[tokio::test]
    async fn an_hours_old_assertion_still_stands_and_says_how_old_it_is() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;

        live.off_console(&session);
        live.last_active_was(&session, Duration::from_secs(3 * 3600));

        let asserted = asserted_by(&live, &session).expect("the assertion expired on its own");
        assert_eq!(asserted.last_active.as_secs(), 3 * 3600);
    }

    /// **Any deliberate act clears it** (v1 §6), and there is one way in rather than one per
    /// act: keying, a subscription, an arm, answering a prompt and dismissing a banner all
    /// arrive here as the same evidence, and so does saying *I am back*.
    #[tokio::test]
    async fn a_deliberate_act_clears_the_assertion_and_restarts_the_age() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;

        live.last_active_was(&session, Duration::from_secs(600));
        live.off_console(&session);
        assert!(asserted_by(&live, &session).is_some());

        live.a_deliberate_act(&session);

        assert_eq!(asserted_by(&live, &session), None, "the assertion outlived");
        // And the clock the *next* assertion is shown against starts from that act rather
        // than from the last one before it.
        live.off_console(&session);
        assert!(
            asserted_by(&live, &session)
                .expect("the assertion")
                .last_active
                < Duration::from_secs(600),
            "an act cleared the claim and left the evidence where it was"
        );
    }

    /// **VoxLoop never guesses whether a human is in the chair** (ADR-0016). A heartbeat is the
    /// machine noticing it can still be reached, so it is not evidence in either direction: it
    /// neither clears an assertion nor refreshes the clock one would be shown against.
    #[tokio::test]
    async fn a_heartbeat_is_not_evidence_that_anybody_is_in_the_chair() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;

        live.last_active_was(&session, Duration::from_secs(900));
        live.off_console(&session);

        live.the_client_is_there(&session);

        let asserted = asserted_by(&live, &session).expect("a heartbeat cleared the claim");
        assert_eq!(
            asserted.last_active.as_secs(),
            900,
            "a heartbeat was read as a person"
        );
    }

    /// **Nothing is inferred from idleness** (ADR-0016). A session nobody has touched for an
    /// hour is a session whose operator is watching telemetry, and the only thing VoxLoop says
    /// about it is how long ago they last did something.
    #[tokio::test]
    async fn a_session_nobody_has_touched_for_an_hour_is_still_on_console() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;

        live.last_active_was(&session, Duration::from_secs(3600));

        assert_eq!(
            asserted_by(&live, &session),
            None,
            "idleness was read as an assertion"
        );
    }

    /// **Declaring it changes nothing else** (v1 §6): subscriptions stand, audio keeps
    /// flowing, and the fan-out does not move at all — so the operator who steps away and
    /// hears something from three metres away still hears it.
    #[tokio::test]
    async fn going_off_console_moves_no_subscription_and_no_route() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let talker = a_session(&live, &store, "flight").await;
        let listener = a_session(&live, &store, "capcom").await;
        let air_to_ground = LoopId::presented("air-to-ground".to_owned());
        let emitting = vec![a_loop_to_emit_on("air-to-ground")];
        live.presence(&talker, emitting.clone(), &[]);
        live.presence(&listener, emitting.clone(), &[]);
        live.arm(&talker, &air_to_ground);
        live.subscribe(&listener, &air_to_ground);
        live.the_client_keys(&talker);
        live.presence(&listener, emitting.clone(), &[]);

        assert_eq!(
            heard_by(&live, &talker),
            vec![(listener.as_str().to_owned(), "air-to-ground".to_owned())],
            "the listener was not hearing the talker to begin with"
        );

        live.off_console(&listener);
        live.presence(&listener, emitting.clone(), &[]);

        assert_eq!(
            live.the_routing_if_it_moved(),
            None,
            "stepping away from the desk moved a route"
        );
        assert!(
            standing_of(&live, &listener, emitting).subscribed,
            "stepping away from the desk dropped a subscription"
        );
    }

    /// **It is never persisted across an assume** (v1 §10). A day-old assertion is not a fact
    /// about anything, so a seat taken up again starts on console.
    #[tokio::test]
    async fn an_assertion_is_gone_at_the_next_assume() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let (sign_in, user, role) = a_seat(&store, "flight", "Flight Director").await;
        let session = live
            .assume(taking(&sign_in, &user, &role, Some(1)))
            .expect("the seat to be free")
            .session;

        live.off_console(&session);
        live.ended_by_its_own_holder(&session);

        let again = live
            .assume(taking(&sign_in, &user, &role, Some(1)))
            .expect("the seat to be free")
            .session;

        assert_eq!(
            asserted_by(&live, &again),
            None,
            "an assertion outlived the session it was made in"
        );
    }

    /// **Off console is one of the reasons an occupant is not hearing a loop** (ADR-0065), and
    /// it sits second: a session that cannot be reached at all explains the silence on its own,
    /// and everything below is still true if the loop were fixed.
    #[tokio::test]
    async fn off_console_is_the_reason_an_occupant_is_not_hearing_a_loop_they_monitor() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;
        let flight = LoopId::presented("flight".to_owned());
        live.presence(&session, vec![a_loop("flight")], &[]);
        live.subscribe(&session, &flight);
        live.the_client_counted(&session, &counted("flight", 12));

        assert_eq!(live.why_not_hearing(&session, &flight), None);

        live.off_console(&session);
        assert_eq!(
            live.why_not_hearing(&session, &flight),
            Some(NotHearing::OffConsole),
            "an operator who said they were away was counted as hearing the loop"
        );

        // It is not the reason for a session nobody can reach: that one is furthest upstream
        // and stands in for every reason below it, assertions included.
        live.unheard_from_for(&session, PAST_THE_WINDOW);
        assert_eq!(
            live.why_not_hearing(&session, &flight),
            Some(NotHearing::Unreachable)
        );
    }

    /// Whoever holds this session, for a test that has to name them to ask whether they still
    /// do.
    fn live_occupant(live: &StateAuthority, session: &SessionId) -> UserId {
        live.read(|held| {
            held.sessions
                .iter()
                .find(|session_held| &session_held.id == session)
                .map(|session_held| session_held.occupant.clone())
                .expect("a session")
        })
    }

    /// Whether a human is behind that loop, as this session's own document has it.
    fn staffing_of(
        live: &StateAuthority,
        session: &SessionId,
        within: Vec<InReach>,
        staffed: &[StaffedBy],
    ) -> Option<Staffing> {
        live.presence(session, within, staffed)
            .expect("a document")
            .1
            .loops
            .into_iter()
            .next()
            .expect("one loop in reach")
            .staffing
    }

    /// One loop and the roles staffing it, as the grid hands the pairs over.
    fn staffed_by(held_on: &str, roles: &[&RoleId]) -> StaffedBy {
        StaffedBy {
            held_on: LoopId::presented(held_on.to_owned()),
            roles: roles.iter().map(|role| (*role).clone()).collect(),
        }
    }

    /// A second person in the same seat, for the counting a multi-occupant role is the whole
    /// case for.
    async fn another_occupant(
        live: &StateAuthority,
        store: &Store,
        who: &str,
        role: &RoleId,
    ) -> SessionId {
        let (sign_in, user, _their_own_role) =
            a_seat(store, who, &format!("{who}'s own role")).await;

        live.assume(taking(&sign_in, &user, role, None))
            .expect("the seat to take another")
            .session
    }

    /// Somebody hearing the loop: it is on their console, its beacon is arriving, and they
    /// have neither muted it nor stepped away.
    fn hearing(live: &StateAuthority, session: &SessionId, held_on: &LoopId) {
        live.presence(session, vec![a_loop(held_on.as_str())], &[]);
        live.subscribe(session, held_on);
        live.the_client_counted(session, &counted(held_on.as_str(), 12));
    }

    /// **A loop with no staffing roles has no staffing state at all** ([ADR-0056]). It is not
    /// `vacant`: two people may be talking on it right now.
    ///
    /// [ADR-0056]: ../../docs/adr/0056-a-loop-with-no-staffing-roles-has-no-staffing-state.md
    #[tokio::test]
    async fn a_loop_nothing_staffs_has_no_staffing_state() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;
        let flight = LoopId::presented("flight".to_owned());
        hearing(&live, &session, &flight);

        assert_eq!(
            staffing_of(&live, &session, vec![a_loop("flight")], &[]),
            None,
            "a loop nobody staffs was given a staffing state"
        );
    }

    /// Removing the last staffing role takes the state away, and that is a configuration
    /// change like any other rather than an error ([ADR-0056]).
    ///
    /// [ADR-0056]: ../../docs/adr/0056-a-loop-with-no-staffing-roles-has-no-staffing-state.md
    #[tokio::test]
    async fn losing_the_last_staffing_role_leaves_the_loop_with_no_staffing_state() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let (sign_in, user, role) = a_seat(&store, "gene", "Flight Director").await;
        let session = live
            .assume(taking(&sign_in, &user, &role, Some(1)))
            .expect("the seat to be free")
            .session;
        let flight = LoopId::presented("flight".to_owned());
        hearing(&live, &session, &flight);
        let staffed = [staffed_by("flight", &[&role])];

        assert_eq!(
            staffing_of(&live, &session, vec![a_loop("flight")], &staffed),
            Some(Staffing::Staffed)
        );
        assert_eq!(
            staffing_of(&live, &session, vec![a_loop("flight")], &[]),
            None
        );
    }

    /// `vacant` is nobody in the seat, and it is materially different from `away`: there is
    /// no console for anybody to fix.
    ///
    /// **A service principal cannot produce anything else**: occupancy has exactly one origin
    /// and it is an assume ([ADR-0005]), which a service principal never makes — its role
    /// binding gives reach and never occupancy ([ADR-0027]).
    ///
    /// [ADR-0005]: ../../docs/adr/0005-occupancy-means-listening-not-signed-in.md
    /// [ADR-0027]: ../../docs/adr/0027-a-service-principal-acts-through-a-role.md
    #[tokio::test]
    async fn a_loop_whose_staffing_roles_nobody_occupies_is_vacant() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;
        let (_sign_in, _user, unoccupied) = a_seat(&store, "gene", "Ground Alarms").await;
        let flight = LoopId::presented("flight".to_owned());
        hearing(&live, &session, &flight);

        assert_eq!(
            staffing_of(
                &live,
                &session,
                vec![a_loop("flight")],
                &[staffed_by("flight", &[&unoccupied])]
            ),
            Some(Staffing::Vacant),
            "a loop nobody occupies a staffing role on was read as covered"
        );
    }

    /// `staffed` means an occupant of a staffing role is **demonstrably hearing** it: the
    /// loop is on their console and its beacon is arriving ([ADR-0017]).
    ///
    /// [ADR-0017]: ../../docs/adr/0017-loop-health-is-measured-not-asserted.md
    #[tokio::test]
    async fn an_occupant_demonstrably_hearing_it_staffs_it() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let (sign_in, user, role) = a_seat(&store, "gene", "Flight Director").await;
        let session = live
            .assume(taking(&sign_in, &user, &role, Some(1)))
            .expect("the seat to be free")
            .session;
        let flight = LoopId::presented("flight".to_owned());
        let staffed = [staffed_by("flight", &[&role])];

        assert_eq!(
            staffing_of(&live, &session, vec![a_loop("flight")], &staffed),
            Some(Staffing::Away(vec![(NotHearing::NotSubscribed, 1)])),
            "a loop nobody has on their console was read as covered"
        );

        hearing(&live, &session, &flight);

        assert_eq!(
            staffing_of(&live, &session, vec![a_loop("flight")], &staffed),
            Some(Staffing::Staffed)
        );
    }

    /// **There is no partial value.** One occupant hearing it is the whole answer, however
    /// many others have stepped away: the question is whether a human is behind the loop.
    #[tokio::test]
    async fn one_occupant_hearing_it_staffs_it_whatever_the_others_are_doing() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let (sign_in, user, role) = a_seat(&store, "gene", "Flight Director").await;
        let hearing_it = live
            .assume(taking(&sign_in, &user, &role, None))
            .expect("the seat to be free")
            .session;
        let away = another_occupant(&live, &store, "flight", &role).await;
        let flight = LoopId::presented("flight".to_owned());
        hearing(&live, &hearing_it, &flight);
        hearing(&live, &away, &flight);
        live.mute(&away, &flight);

        assert_eq!(
            staffing_of(
                &live,
                &hearing_it,
                vec![a_loop("flight")],
                &[staffed_by("flight", &[&role])]
            ),
            Some(Staffing::Staffed)
        );
    }

    /// **The reason is a count over occupants and it ranks nothing** ([ADR-0065]): a mute is
    /// one click from hearing and so is a subscription, so no ordering across people is
    /// defensible. The counts arrive furthest upstream first, which is an order to read them
    /// in rather than a precedence.
    ///
    /// [ADR-0065]: ../../docs/adr/0065-the-staffing-flag-reports-it-never-subscribes.md
    #[tokio::test]
    async fn away_counts_every_occupant_by_reason() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let (sign_in, user, role) = a_seat(&store, "gene", "Flight Director").await;
        let muted = live
            .assume(taking(&sign_in, &user, &role, None))
            .expect("the seat to be free")
            .session;
        let unsubscribed = another_occupant(&live, &store, "flight", &role).await;
        let also_unsubscribed = another_occupant(&live, &store, "capcom", &role).await;
        let flight = LoopId::presented("flight".to_owned());
        hearing(&live, &muted, &flight);
        live.mute(&muted, &flight);
        live.presence(&unsubscribed, vec![a_loop("flight")], &[]);
        live.presence(&also_unsubscribed, vec![a_loop("flight")], &[]);

        assert_eq!(
            staffing_of(
                &live,
                &muted,
                vec![a_loop("flight")],
                &[staffed_by("flight", &[&role])]
            ),
            Some(Staffing::Away(vec![
                (NotHearing::NotSubscribed, 2),
                (NotHearing::Muted, 1),
            ]))
        );
    }

    /// **Within one occupant the reason is the one furthest upstream** — the one still true
    /// if everything below it were fixed — so somebody unreachable is counted once and as
    /// unreachable, whatever else is also true of them (v1 §8).
    #[tokio::test]
    async fn one_occupant_is_counted_once_under_the_reason_furthest_upstream() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let (sign_in, user, role) = a_seat(&store, "gene", "Flight Director").await;
        let session = live
            .assume(taking(&sign_in, &user, &role, None))
            .expect("the seat to be free")
            .session;
        let flight = LoopId::presented("flight".to_owned());
        hearing(&live, &session, &flight);
        live.mute(&session, &flight);
        live.off_console(&session);
        live.unheard_from_for(&session, PAST_THE_WINDOW);

        assert_eq!(
            staffing_of(
                &live,
                &session,
                vec![a_loop("flight")],
                &[staffed_by("flight", &[&role])]
            ),
            Some(Staffing::Away(vec![(NotHearing::Unreachable, 1)])),
            "one occupant was counted under more than one reason at once"
        );
    }

    /// It is computed across **every** staffing role, not only the one the reader is in.
    #[tokio::test]
    async fn staffing_state_reads_every_occupant_of_every_staffing_role() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let (sign_in, user, flight_director) = a_seat(&store, "gene", "Flight Director").await;
        let watching = live
            .assume(taking(&sign_in, &user, &flight_director, Some(1)))
            .expect("the seat to be free")
            .session;
        let (theirs, them, capcom) = a_seat(&store, "flight", "CAPCOM").await;
        let hearing_it = live
            .assume(taking(&theirs, &them, &capcom, Some(1)))
            .expect("the seat to be free")
            .session;
        let flight = LoopId::presented("flight".to_owned());
        hearing(&live, &hearing_it, &flight);
        live.presence(&watching, vec![a_loop("flight")], &[]);

        assert_eq!(
            staffing_of(
                &live,
                &watching,
                vec![a_loop("flight")],
                &[staffed_by("flight", &[&flight_director, &capcom])]
            ),
            Some(Staffing::Staffed),
            "an occupant of the other staffing role was left out of the answer"
        );
    }

    /// **The mark is a fact about this console's own configuration**, carried whether or not
    /// anything is wrong: the second state the console draws is this field beside the
    /// subscription it already has (v1 §8, ADR-0065).
    #[tokio::test]
    async fn the_document_says_whether_this_sessions_role_staffs_each_loop() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let (sign_in, user, role) = a_seat(&store, "gene", "Flight Director").await;
        let session = live
            .assume(taking(&sign_in, &user, &role, Some(1)))
            .expect("the seat to be free")
            .session;
        let (_theirs, _them, somebody_else) = a_seat(&store, "flight", "CAPCOM").await;

        let marked = live
            .presence(
                &session,
                vec![a_loop("flight"), a_loop("air-to-ground")],
                &[
                    staffed_by("flight", &[&role]),
                    staffed_by("air-to-ground", &[&somebody_else]),
                ],
            )
            .expect("a document")
            .1;

        assert!(
            marked.loops[0].staffs,
            "the loop this role staffs was unmarked"
        );
        assert!(
            !marked.loops[1].staffs,
            "a loop somebody else's role staffs was marked on this console"
        );
    }

    /// The audience of this session's arm set, as its own document carries it.
    fn audience_of(live: &StateAuthority, session: &SessionId, within: Vec<InReach>) -> Audience {
        live.presence(session, within, &[])
            .expect("a document")
            .1
            .audience
    }

    /// Both loops, on a row that may speak on either.
    ///
    /// The reach is handed back to every projection, and an arm outside it is taken away
    /// ([ADR-0013]) — so a test asking about an arm set of two has to hand back the reach that
    /// holds both of them, or it is asking about an arm set of one.
    ///
    /// [ADR-0013]: ../../docs/adr/0013-arming-is-independent-of-subscription.md
    fn both_to_emit_on() -> Vec<InReach> {
        vec![a_loop_to_emit_on("air-to-ground"), a_loop_to_emit_on("sim")]
    }

    /// A talker armed on both loops, and a listener in reach of both and monitoring neither,
    /// so that a question about one person hearing two destinations has two destinations to be
    /// asked about and nothing taken up in advance.
    async fn a_talker_armed_on_two_loops(
        live: &StateAuthority,
        store: &Store,
    ) -> (SessionId, SessionId) {
        let talker = a_session(live, store, "flight").await;
        let listener = a_session(live, store, "capcom").await;
        let reach = vec![a_loop_to_emit_on("air-to-ground"), a_loop_to_emit_on("sim")];

        live.presence(&talker, reach.clone(), &[]);
        live.presence(&listener, reach, &[]);
        for held_on in ["air-to-ground", "sim"] {
            live.arm(&talker, &LoopId::presented(held_on.to_owned()));
        }

        (talker, listener)
    }

    /// **The audience is per person and never per destination** (v1 §6). The fan-out counts a
    /// listener once per loop, because the recording tap is per (talker, destination); the bar
    /// answers *how many people will hear me*, and one colleague on two of the armed loops is
    /// one person.
    #[tokio::test]
    async fn somebody_hearing_two_armed_loops_is_one_person_hearing() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let (talker, listener) = a_talker_armed_on_two_loops(&live, &store).await;

        for held_on in ["air-to-ground", "sim"] {
            let held_on = LoopId::presented(held_on.to_owned());
            live.subscribe(&listener, &held_on);
            live.the_client_counted(&listener, &counted(held_on.as_str(), 12));
        }

        assert_eq!(
            audience_of(&live, &talker, both_to_emit_on()),
            Audience {
                hearing: 1,
                present_not_hearing: 0,
                not_subscribed: 0
            },
            "one listener on two armed loops was counted twice"
        );
    }

    /// **Hearing one of them is hearing.** The question the bar answers is *will my voice
    /// reach this person*, and a colleague who has one of the armed loops up will hear it
    /// however many of the others they have not.
    #[tokio::test]
    async fn hearing_one_armed_loop_out_of_two_is_hearing() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let (talker, listener) = a_talker_armed_on_two_loops(&live, &store).await;
        let sim = LoopId::presented("sim".to_owned());

        live.subscribe(&listener, &sim);
        live.the_client_counted(&listener, &counted(sim.as_str(), 12));

        assert_eq!(
            audience_of(&live, &talker, both_to_emit_on()).hearing,
            1,
            "somebody monitoring one of the armed loops was not counted as hearing"
        );
    }

    /// **`present, not hearing` is the console's only warning about mute** (ADR-0034). The
    /// subscription stands, so this person believes they are covering the loop; the audio
    /// stops in their ears alone, and nothing else on the console says so.
    #[tokio::test]
    async fn a_subscriber_who_muted_the_loop_is_present_and_not_hearing() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let (talker, listener) = a_talker_armed_on_two_loops(&live, &store).await;
        let air_to_ground = LoopId::presented("air-to-ground".to_owned());

        live.subscribe(&listener, &air_to_ground);
        live.the_client_counted(&listener, &counted(air_to_ground.as_str(), 12));
        live.mute(&listener, &air_to_ground);

        assert_eq!(
            audience_of(&live, &talker, both_to_emit_on()),
            Audience {
                hearing: 0,
                present_not_hearing: 1,
                not_subscribed: 0
            },
            "a mute was not warned about"
        );
    }

    /// Off console is the one asserted reason among observed ones (ADR-0016), and it counts
    /// here like the rest: this person is not in the chair, and their subscription says they
    /// meant to be covering the loop.
    #[tokio::test]
    async fn a_subscriber_who_stepped_away_is_present_and_not_hearing() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let (talker, listener) = a_talker_armed_on_two_loops(&live, &store).await;
        let air_to_ground = LoopId::presented("air-to-ground".to_owned());

        live.subscribe(&listener, &air_to_ground);
        live.the_client_counted(&listener, &counted(air_to_ground.as_str(), 12));
        live.off_console(&listener);

        assert_eq!(
            audience_of(&live, &talker, both_to_emit_on()).present_not_hearing,
            1,
            "somebody who said they were away was counted as hearing"
        );
    }

    /// **A session with no signalling channel has its fan-out closed** (ADR-0018), so nothing
    /// reaches it whatever it last subscribed to.
    #[tokio::test]
    async fn a_subscriber_nobody_can_reach_is_present_and_not_hearing() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let (talker, listener) = a_talker_armed_on_two_loops(&live, &store).await;
        let air_to_ground = LoopId::presented("air-to-ground".to_owned());

        live.subscribe(&listener, &air_to_ground);
        live.the_client_counted(&listener, &counted(air_to_ground.as_str(), 12));
        live.unheard_from_for(&listener, PAST_THE_WINDOW);

        assert_eq!(
            audience_of(&live, &talker, both_to_emit_on()).present_not_hearing,
            1,
            "a session nobody can reach was counted as hearing"
        );
    }

    /// **Beacon loss soundly proves deafness** (ADR-0017): this console is subscribed to the
    /// loop and demonstrably receiving nothing on it, which is the case a subscriber list
    /// cannot tell from a quiet loop.
    #[tokio::test]
    async fn a_subscriber_the_beacon_is_not_reaching_is_present_and_not_hearing() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let (talker, listener) = a_talker_armed_on_two_loops(&live, &store).await;
        let air_to_ground = LoopId::presented("air-to-ground".to_owned());

        live.subscribe(&listener, &air_to_ground);
        live.the_beacons_went_unheard_for(&listener, PAST_THE_WINDOW);

        assert_eq!(
            audience_of(&live, &talker, both_to_emit_on()).present_not_hearing,
            1,
            "a console receiving nothing on the loop was counted as hearing"
        );
    }

    /// **The third bucket is the people who chose not to listen, and it is never the
    /// warning** (ADR-0034). It is computed — the three-way split is the point — and it is
    /// what the count that *is* shown would silently swallow if the split were two-way.
    ///
    /// The listener here has stepped away as well, which is deliberate: an assertion from
    /// somebody who never took the loop up is not a warning about a colleague who believes
    /// they are covering it, so it does not become one.
    #[tokio::test]
    async fn somebody_who_never_took_the_loop_up_is_the_third_bucket_and_not_the_warning() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let (talker, listener) = a_talker_armed_on_two_loops(&live, &store).await;
        live.off_console(&listener);

        assert_eq!(
            audience_of(&live, &talker, both_to_emit_on()),
            Audience {
                hearing: 0,
                present_not_hearing: 0,
                not_subscribed: 1
            },
            "somebody who did not subscribe was warned about"
        );
    }

    /// Hearing yourself back over the network is a fault in an intercom, and counting
    /// yourself is the same fault arriving as a number.
    #[tokio::test]
    async fn the_audience_never_counts_the_talker_themselves() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let talker = a_session(&live, &store, "flight").await;
        let air_to_ground = LoopId::presented("air-to-ground".to_owned());
        live.presence(&talker, vec![a_loop_to_emit_on("air-to-ground")], &[]);
        live.arm(&talker, &air_to_ground);
        live.subscribe(&talker, &air_to_ground);
        live.the_client_counted(&talker, &counted(air_to_ground.as_str(), 12));

        assert_eq!(
            audience_of(&live, &talker, vec![a_loop_to_emit_on("air-to-ground")]),
            Audience {
                hearing: 0,
                present_not_hearing: 0,
                not_subscribed: 0
            },
            "the talker was in their own audience"
        );
    }

    /// **The three buckets partition the people in reach of the armed set** (v1 §6). Somebody
    /// whose role does not reach a loop is not a person who chose not to listen: they were
    /// never asked, and a count that included them would answer a question nobody put.
    #[tokio::test]
    async fn nobody_out_of_reach_of_the_armed_set_is_in_any_bucket() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let talker = a_session(&live, &store, "flight").await;
        let elsewhere = a_session(&live, &store, "capcom").await;
        live.presence(&talker, vec![a_loop_to_emit_on("air-to-ground")], &[]);
        live.presence(&elsewhere, vec![a_loop("sim")], &[]);
        live.arm(&talker, &LoopId::presented("air-to-ground".to_owned()));

        assert_eq!(
            audience_of(&live, &talker, vec![a_loop_to_emit_on("air-to-ground")]),
            Audience {
                hearing: 0,
                present_not_hearing: 0,
                not_subscribed: 0
            },
            "somebody the grid does not let near the loop was counted"
        );
    }

    /// An arm set with nothing in it reaches nobody, and the bar says `0 hearing` rather than
    /// nothing at all: it renders in the warning colour and blocks no transmission
    /// (ADR-0034).
    #[tokio::test]
    async fn an_empty_arm_set_has_nobody_in_any_bucket() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let (talker, listener) = a_talker_armed_on_two_loops(&live, &store).await;
        let air_to_ground = LoopId::presented("air-to-ground".to_owned());
        live.subscribe(&listener, &air_to_ground);
        live.the_client_counted(&listener, &counted(air_to_ground.as_str(), 12));

        for held_on in ["air-to-ground", "sim"] {
            live.disarm(&talker, &LoopId::presented(held_on.to_owned()));
        }

        assert_eq!(
            audience_of(&live, &talker, both_to_emit_on()).hearing,
            0,
            "an arm set with nothing in it had an audience"
        );
    }

    /// **The bar stays live while keyed** (ADR-0058), and it answers *who am I about to talk
    /// to* and *who am I talking to* with the same words — so the audience is the same answer
    /// computed the same way, before the key goes down and while it is held.
    #[tokio::test]
    async fn the_audience_is_the_same_answer_before_the_key_and_under_it() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let (talker, listener) = a_talker_armed_on_two_loops(&live, &store).await;
        let air_to_ground = LoopId::presented("air-to-ground".to_owned());
        live.subscribe(&listener, &air_to_ground);
        live.the_client_counted(&listener, &counted(air_to_ground.as_str(), 12));

        let before = audience_of(&live, &talker, both_to_emit_on());
        live.the_client_keys(&talker);

        assert_eq!(
            audience_of(&live, &talker, both_to_emit_on()),
            before,
            "keying changed the answer the operator read a moment before"
        );
    }

    /// **Only a change the session did not ask for is marked** (ADR-0058). An administrator
    /// pulling an `emit` cell is the change no hand on this desk made, and the operator may be
    /// mid-sentence when it lands.
    #[tokio::test]
    async fn an_arm_a_revocation_took_is_marked_on_the_document() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;
        let emitting = vec![a_loop_to_emit_on("air-to-ground")];
        live.presence(&session, emitting.clone(), &[]);
        live.arm(&session, &LoopId::presented("air-to-ground".to_owned()));

        let (_, unmarked) = live.presence(&session, emitting, &[]).expect("a document");
        assert!(
            !unmarked.arms_moved_elsewhere,
            "an arm set nobody has touched was marked as moved"
        );

        // The cell goes to `monitor`: the loop is still in reach and the arm is gone.
        let (_, marked) = live
            .presence(&session, vec![a_loop("air-to-ground")], &[])
            .expect("a document");

        assert!(
            marked.arms_moved_elsewhere,
            "a revocation took an arm and said nothing"
        );
    }

    /// **A change the operator's own hand made is not marked** (ADR-0058). A deliberate arm
    /// and a preset are the routine mid-key changes, and marking them would fire the signal
    /// constantly and train the operator straight past it.
    #[tokio::test]
    async fn the_operators_own_disarm_is_not_marked() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;
        let emitting = vec![a_loop_to_emit_on("air-to-ground")];
        let air_to_ground = LoopId::presented("air-to-ground".to_owned());
        live.presence(&session, emitting.clone(), &[]);
        live.arm(&session, &air_to_ground);
        live.disarm(&session, &air_to_ground);

        let (_, presence) = live.presence(&session, emitting, &[]).expect("a document");

        assert!(
            !presence.arms_moved_elsewhere,
            "the operator was told their own disarm was somebody else's"
        );
    }

    /// The mark stands until the operator does something deliberate, which is the same rule
    /// the one asserted state is cleared by (ADR-0016): an act on this console is the evidence
    /// that the person at it has seen what is on it.
    #[tokio::test]
    async fn a_deliberate_act_clears_the_mark() {
        let (_directory, store) = a_temporary_store().await;
        let live = StateAuthority::empty();
        let session = a_session(&live, &store, "flight").await;
        live.presence(&session, vec![a_loop_to_emit_on("air-to-ground")], &[]);
        live.arm(&session, &LoopId::presented("air-to-ground".to_owned()));
        live.presence(&session, vec![a_loop("air-to-ground")], &[]);

        live.a_deliberate_act(&session);

        let (_, presence) = live
            .presence(&session, vec![a_loop("air-to-ground")], &[])
            .expect("a document");

        assert!(
            !presence.arms_moved_elsewhere,
            "the mark outlived the act that answered it"
        );
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
