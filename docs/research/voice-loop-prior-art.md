# Prior art: existing voice loop and control-room intercom systems

Research note for [issue #4](https://github.com/edwardhutchinson/voxloop/issues/4). Resolves the question:
*what do existing voice loop systems actually do, and which of their primitives are worth borrowing?*

**Bottom line.** Across space operations, dispatch, trading floors, broadcast and air traffic control,
the same small set of primitives recurs, and the recurrence is not coincidence — they are the primitives
that make a many-channel audio environment survivable. The most important of them is a **three-term
vocabulary that VoxLoop currently collapses into one word**: a *user* is a person, a *role* is a staffable
position, and a *loop* is an audio conference. Every mature system separates these. VoxLoop's terminology
problem is not an unsolved design question — it is a solved one that VoxLoop has not yet adopted.

The second most important is **two-stage emission**: you *arm* a destination, then you *key* it. These are
separate acts with separate indicators, and conflating them is how operators end up talking to the wrong
people.

Everything else — per-source volume, monitor/talk colour coding, latch, priority, interrupt — is real but
secondary. The parts that are genuinely legacy artefacts are enumerated in [§6](#6-do-not-copy-these).

---

## 1. Method and source quality

Claims below cite primary sources inline: vendor manuals and specification sheets, agency documentation,
published operational handbooks, and one peer-reviewed field study conducted under a NASA grant. Where only
secondary or indirect sourcing was available, the claim is marked.

**Confidence ledger.**

| Source | Type | Confidence | Note |
|---|---|---|---|
| Patterson, Watts-Perotti & Woods, *Voice Loops as Coordination Aids in Space Shuttle Mission Control* | Peer-reviewed field study, 130+ hours direct observation, NASA Grant NAG9-390 | **High** | Shuttle-era (published 1999). Describes practice, not a spec. Still the single best account of *why* loops work. |
| NASA MSFC, *Voice over Internet Protocol (VoIP) in a Control Center Environment* (NTRS 20100020881) | NASA conference paper by the system owners | **High** | Gives the actual requirements list for a NASA mission voice system. |
| DLR GSOC **openvocs** handbook, README and client API (GitHub, Apache-2.0) | Vendor/author primary documentation + source | **High** | The closest existing analogue to VoxLoop. Current (2025). |
| Motorola *MCC 7000 Series Dispatch Consoles: Elite Dispatch End User Toolkit* (ACS7173601) | Vendor operator training manual | **High** | |
| Speakerbus *ARIA Hoot User Guide* (rev 5) | Vendor user manual | **High** | |
| Clear-Com *i-Series Panels User Guide* (399G207A) | Vendor user manual | **High** | |
| RTS *Digital Matrix Intercom System* product documentation | Vendor product documentation | **High** | Terminology only; brochure-level detail. |
| Trilogy *Gemini Digital Intercom Implementation Guide* v3.4a | Vendor manual | **High** | |
| Rohde & Schwarz *R&S®VCS-4G* and *CERTIUM VCS* product brochures | Vendor specification literature | **High** for feature existence, **Medium** for operational detail | Marketing register; describes capability, not procedure. |
| ESA public pages and ESA operations blogs | Official agency, journalistic register | **Medium** | ESA does not publish its voice system's operational documentation. Confirms vocabulary and practice; gives no console detail. |
| Cisco IPICS administration and dispatch console guides | Vendor primary docs, **reached only via search-engine excerpts** — cisco.com returned HTTP 403 to every direct fetch | **Medium** | Quotes are from the official docs as indexed; not independently verified against the live page. Flagged inline. |
| EUROCAE ED-137 (VoIP for ATM) | Standard, **paywalled** | **Low–Medium** | Priority model described from derived sources only. Flagged inline. |
| FAA Order JO 7210.3, paras 2-2-4 and 2-2-6 | Agency order | **Medium** | Paragraph titles and subject matter verified via citation in other official FAA documents; full text not read. |

**What I could not reach.** Cisco IPICS documentation (403 to all fetch attempts), EUROCAE ED-137 (paywalled),
any ESA/ESOC internal voice system specification (not public), and NASA JSC flight controller console handbooks
(not public). No claim below rests on model recall.

---

## 2. System-by-system

### 2.1 NASA Mission Control (Shuttle / ISS)

The canonical implementation, and the origin of the word "loop".

> "a voice loop is a real-time auditory channel that connects physically distributed people. A controller who
> speaks on a loop broadcasts to all controllers who are listening in on that loop."
> — Patterson et al., §4.1

Scale and shape:

- **164 voice loops were in use during a single mission** (STS-76). A controller typically monitors **four in
  parallel**: the Flight Director loop, the Air-to-Ground loop, their Front-to-Back support loop, and a
  conference loop (Patterson et al., §4.3).
- Loops are **typed by function**, and the type determines the etiquette. The Flight Director loop is
  monitored by everyone but only front-room controllers may speak on it, and "only issues of high significance
  are discussed on it". Air-to-Ground is monitored by everyone but only CAPCOM may transmit. Front-to-Back
  loops carry detailed, informal work. Conference loops are "continuously monitored but lie unused until a
  situation arises that requires coordination across subsystem controllers" (Patterson et al., §4.2).
- Loops are **pre-provisioned, not created on demand**. Patterson et al. argue this explicitly, and the argument
  is directly relevant to VoxLoop's open "breakout channels" question — see [§5.8](#58-pre-provision-loops-do-not-make-operators-create-them-under-load).

Console mechanics:

- "each channel can be set either to monitor or talk modes. **Only one channel at a time can be set to the
  talk mode**, although many channels can be monitored at the same time." (§4.1)
- Talking is a **second, separate act**: "In order to talk on a loop set to the talk mode, a controller presses
  a button on a hand unit or holds down a foot pedal and talks into a headset." (§4.1)
- The interface is "a map of the available loops" — a spatial grid, directly manipulated. Controllers save
  configurations as **'pages'** under their identification code, with the most-used loops on a primary page (§4.1).
- Per-loop volume is a first-class attention tool: controllers "reorganize and prioritize the loops to fit the
  particular operational situation … by changing the configuration of loops that are being monitored and by
  adjusting the relative volume levels on each loop", and "increase or decrease the salience of particular loops
  by using loop volume controls to adjust relative loudness" (§4.1).

Protocol, not machinery, does the priority work:

- Addressing is **by console position, never by name**: "The standard protocol for initiating communications on
  the loops is to name the person that you wish to speak to in order to get his or her attention and then
  identify yourself (e.g., "Flight, MMACS")." The callee replies "Standby" or "Go ahead" (§5.3).
- Forced injection is a **verbal convention**: "In crisis situations, controllers will use the Flight Director
  loop to broadcast critical information after declaring "Break! Break!" … an explicit protocol that is rarely
  used and would instantly gain the attention of mission controllers." (§5.3)
- Interruptibility is judged by **listening first**: a controller who needs another team will first monitor that
  team's Front-to-Back loop to estimate their workload before interrupting (§5.3). This is the single most
  important behaviour in the whole paper and it is enabled by one property: you can listen without announcing
  yourself. "controllers can listen in on communications without disrupting or even alerting the participants"
  (§7).

### 2.2 NASA MSFC HOSC — EVoDS / IVoDS

*Pirani & Calvelage, "Voice over Internet Protocol (VoIP) in a Control Center Environment", NTRS 20100020881.*
This is the closest thing to a published NASA requirements list for a mission voice system.

- Scale: "The POIC supports 100 concurrent users accessing up to 500 unique conferences"; the requirements table
  lists **"950 real time conferences"**.
- Requirements table includes: *Access control to restricted conferences; Individual user IDs; Individual user
  preferences; LDAP interoperable; Multiple headsets, with external Speakers and Foot Switches; Sidetone; Gain and
  attenuation controls; Non blocking; Rapid reconfiguration; System alarm and status.*
- The defining behavioural difference from telephony is stated outright: "Mission Voice Conferencing systems allow
  users access to hundreds of conferences at the push of a button: **no dialing, no call set up, no waiting**."
- Personalisation is per-user and follows the login: user settings "included loop location on the key frame matrix,
  individual volume control … These customized configurations are saved for each user as the keyset and system is
  accessed through individual login."
- PTT is hardware: "a traditional pistol grip style with integrated push-to-talk (PTT) function."

### 2.3 ESA / ESOC

ESA does not publish operational voice system documentation. What is confirmed from ESA's own pages:

- The term is "voice loop", used exactly as NASA uses it. "Every team communicate with each other via voice loop
  and **all communications are recorded**." ([ESA blogs, *ESOC readies for Sentinel-5P*, 2017](https://blogs.esa.int/eolaunches/2017/09/26/esoc-readies-for-sentinel-5p/))
- Readiness polling runs on the loop: "each engineer on console confirms to the Flight Operations Director their
  personal readiness … Once the internal ESOC roll call is complete, the Flight Director reports the centre's
  readiness via the voice loop to the Launch Control Centre." ([ESA, *Countdown*](https://www.esa.int/About_Us/ESOC/Countdown))
- Positions have **call signs used on the loop** — the Flight Operations Director is "the 'OD' … on the voice loop"
  ([ESA blogs](https://blogs.esa.int/eolaunches/2017/09/26/esoc-readies-for-sentinel-5p/)); ESA describes the
  Flight Operations Director as sitting on console in the MCR and Spacecraft Operations Managers as distinct
  positions. **Confidence: medium** — these are agency-published articles, not procedures.
- Ground station operators use the loop to reach flight control teams, and during LEOP the Network Operations
  Centre "is in full-time voice communications with the mission controllers in ESOC's Main Control Room (MCR)",
  with NOC operators "directly managed by the Ground Operations Manager, who occupies a console seat in the MCR"
  ([ESA, *Network Operations Centre*](https://www.esa.int/Enabling_Support/Operations/ESA_Ground_Stations/Network_Operations_Centre)).

The originating brief's observation that loop entities are "an instance like a ground station or a support
engineer, and not necessarily the engineer themselves" is corroborated: the loop identity is the *position*
(OD, SOM, Ground Station X), and a human occupies it.

### 2.4 DLR GSOC — **openvocs** (the closest existing analogue to VoxLoop)

This is the most directly relevant prior art found, and it deserves to be read as more than research material:
it is an existing, current, Apache-2.0, self-hostable, browser-based software voice loop system built
specifically for space mission control by the German Space Operations Center.
([openvocs on GitHub](https://github.com/openvocs/openvocs);
[DLR announcement](https://www.dlr.de/en/rb/latest/news/2025/gsoc-publishes-open-source-software-platform-openvocs-for-communication-in-mission-control))

Its own description of itself resolves VoxLoop's terminology problem in one sentence:

> "the voice communication is organized via **voice loops** (multiple audio conferences held in parallel).
> Further, the system provides a **role-based access control**, where depended on specific user roles, the access
> to voice loops is restricted. Moreover, **the allowance to only monitor or talk in a voice loop is
> differentiated**."
> — openvocs README / User Handbook v1.2, §2

The data model is `user → role → loop`, and it is visible in the client API:

| Event | Parameters | What it establishes |
|---|---|---|
| `user_roles` | — | A user has many roles |
| `update_login` (authorize) | `role` | You **sign in as a role**, not just as yourself |
| `role_loops` | — | A role has a set of loops |
| `switch_state` | `loop`, `state: "recv" \| "send" \| "none"` | Per-loop **tri-state**, one field |
| `switch_ptt` | `loop`, `state: true \| false` | Keying is **separate** from arming |
| `switch_volume` | `loop`, `volume: 0..100` | Per-loop volume is a first-class operation |

Client behaviour, from the User Handbook §3.1:

- Login → **role selection** ("select the role or project … only the roles and projects are shown that are allowed
  for the user"). "The voice loops associated with the role/project **and the corresponding layout** is loaded
  automatically." Layouts are authored per role in the admin client.
- Each loop is a tile. Tri-state cycle by tapping the tile: grey (off) → **blue (monitoring)** → **green (talk)** →
  blue → …, with a separate OFF button.
- **Permission is drawn on the tile**: "Headphones only => only monitoring of the loop is allowed. Headphones and
  microphone => talking is also allowed in this loop." An unavailable action is visible before it is attempted.
- "It is possible to select several voice loops for monitoring mode, but **only one for talking mode**. If a new
  voice loop is selected for talking mode, the former voice loop will switch back to monitoring automatically."
  — the same constraint NASA's console enforces, arrived at independently.
- Talking requires the tile to be green **and** the PTT bar pressed. Four separate confirmations of an open mic
  are specified: PTT bar turns green; a live waveform of the mic signal; "the **name/role of the user** that has an
  open microphone will appear under the tile name"; a white frame indicates voice traffic on a loop.
- Presence: "Icon with number => shows the **number of active users in the loop**."
- Per-loop volume slider, 0–100, default 50.
- Admin client edits users, roles and loops with the links between them "graphically indicated", plus LDAP import
  and JSON config export.

Note for the project: openvocs occupies a large part of VoxLoop's stated niche (self-hosted, browser-based,
mission-control voice loops, role-based permissions, recorder service, SIP gateway). This is a fact worth
knowing before v1 scope is locked, independent of this research question.

### 2.5 Motorola MCC 7000 / 7500 dispatch consoles

*Motorola, "MCC 7000 Series Dispatch Consoles: Elite Dispatch End User Toolkit", ACS7173601.*

Vocabulary is different and revealing: the unit of subscription is a **resource** (a channel or talkgroup), not a
loop, and the console's central distinction is **selected vs unselected**.

- Selected resources route to the headset / **select speaker**; everything else routes to the **unselect
  speaker(s)**. Answering a call moves audio "from the Resource unselect audio destination to its select audio
  destination (speaker or headset)". `All Mute` "Silences all unselected audio for a preprogrammed time period".
- Per-resource **Volume Control**: "Slide control to increase or decrease the volume of the resource's audio."
- Two emission verbs. **Instant Transmit**: "Press and hold this button to send an Instant Transmit to the
  resource. Changes to red during transmission." **General Transmit** goes to whatever is currently selected:
  "General transmit allows the operator to talk to selected radio resources. Anyone monitoring the same resources
  may also hear the transmission."
- Grouping is explicit and typed:
  - **MSEL (Multiselect)** — "the Dispatch Operator can transmit on all of them simultaneously **but the resources
    cannot communicate with each other**."
  - **Patch** — "A group of resources that can receive messages from the console and transmit to all other members
    of the Patch group."
  - **Primary** — "Radio resources designated by Administrators and Dispatch Operators with a **higher handling
    priority**", marked with a diamond.
- Emission state is **broadcast to peers**: "Resource Transmit Status … A parallel console position that attempts
  to transmit also causes these messages to be sent to all console positions that are assigned to or monitor that
  same resource."
- Emergency handling overrides user preference: on Emergency Alarm / Call / Man Down, "**The volume of the resource
  is automatically set to maximum**. The resource border flashes red." It is cleared explicitly with `Knockdown`,
  and "The emergency is cleared at all operator positions monitoring the resource."

The same select/unselect + per-line volume + line PTT architecture appears in the land mobile radio dispatch
console patent literature (e.g. US 9,641,988), confirming it as the domain's standard shape rather than one
vendor's choice.

### 2.6 Cisco IPICS

**Confidence: medium — cisco.com returned HTTP 403 to every direct fetch; the following is quoted from Cisco's
official administration and dispatch console guides as indexed by search, not verified against the live page.**

Vocabulary: **channel**, **talk group**, **virtual talk group (VTG)**, **participant**, **dispatcher**, **ops view**.
A VTG is the ad-hoc conference construct: "Virtual talk groups (VTG) are used to allow participants to interact with
other participants in a Cisco IPICS incident, channel, virtual talk group (VTG), and radio." The **dispatcher** is a
distinct role: "responsible for setting up virtual talk groups (VTGs), activating VTGs to begin conferences, and
adding or removing members or participants … A dispatcher also monitors active VTGs … and can mute and unmute IDC
users, as necessary."

- **Latch** is a per-user, per-channel *permission*, not just a mode: "Cisco IPICS lets you enable or disable the
  latch feature for an IDC user in the User Details window", and "When Allow Latch is set to true on a channel/VTG,
  the user can use latch". Purpose: "if a user wants to lock in a channel **or talk on multiple channels at the same
  time**."
- Listen-only is rendered graphically: "The PTT area displays diagonal lines when this resource is configured as
  listen-only or your Cisco IPICS user account is configured as listen only."
- Talker identity: "When audio is received, the PTT area may show the **talker ID** (either the Cisco IPICS user ID
  or the radio unit ID)."
- **Ops view** is a scoping construct over resources — relevant to VoxLoop's "permission vectors are hard to
  comprehend" problem, as it is the industry's answer: don't show an operator the whole system, show them a view.

### 2.7 Speakerbus ARIA Hoot (trading floors)

*Speakerbus, "ARIA Hoot User Guide", rev 5.* Different industry, same shape — and the most permissive emission
model of any system surveyed, which makes its warnings instructive.

- Terminology: a **hoot** is a broadcast circuit; the subscription unit is a **speaker channel**; an **open hoot**
  or **shout down** is "a permanent open circuit between two or more parties. Any user can speak at any time and be
  heard over the system immediately" ([Speakerbus, *Trader Voice 101*](https://www.speakerbus.com/blog/trader-voice-101-a-beginners-guide-to-the-basics)).
- Both emission modes are named explicitly and are user-selectable per key: **"Push To Talk"** and **"Push To Latch"**.
  The group-talk softkey defaults to latched and can be reconfigured "as unlatched (press to talk)".
- Permission is on the tile: "To talk on a speaker channel you must have talk permission to that voice service.
  **Listen only speaker channels display the Listen-Only State icon.**"
- The multi-destination hazard is stated as a warning in the manual: "**Latching open more than one speaker channel
  at the same time will cause your voice to be heard on all open speakers.**"
- Multi-destination emission is a deliberate feature with a limit: "The group talk option is used to assign up to
  **eight** speaker channels to the group talk softkey. When the group talk softkey is selected all linked speaker
  channels become active allowing talking to several speaker channels simultaneously." The manual notes the
  alternative — holding several keys at once — "can prove to be cumbersome … particularly … if required to talk to
  four or more speaker channels at the same time."
- Attention controls: per-channel **volume slider**, per-channel **mute**, **master mute**, **VAD indicators**, and
  **Solo mode** — "To mute all speaker channel volumes apart from the selected speaker … The speaker channels Solo
  Mode softkey will turn green. All other channels turn red (muted)."
- Override: "**Barge in** a busy elsewhere call."

### 2.8 Broadcast talkback — Clear-Com, RTS, Trilogy

The oldest and most refined vocabulary, and the source of the colour conventions.

**Clear-Com** (*i-Series Panels User Guide*, 399G207A):

- Latch vs momentary are defined per key: non-latching — "the key remains active for talk or listen for as long as
  you hold down the key"; latching — "allows you to lock a key into place, so that you can talk or listen hands-free.
  Quickly tap a key to latch it … The key will stay latched until you tap it again."
- **Colour is semantic and universal in this domain**: keys light "Bright red, for a talk or talk-with-listen key"
  and "Bright green, for a listen key". Talk and listen are *separate keys* on the same destination.
- Per-source listen level exists and is a rotary control — and, tellingly, so does a global escape hatch:
  **"Reset Listen Levels"** resets "all listen keys back to the default level (the highest possible volume)".
- **Local Exclusive**: "all previously latched keys on your panel deactivate temporarily while you talk exclusively
  to one destination or listen to one source."
- Clear-Com's own glossary defines **ISO** as "panel ISOlation … allows a panel operator to call a destination and
  **interrupt all of that destination's other audio paths** and establish a private conversation", and **IFB**
  ("Interruptible Fold Back") as an arrangement where a program source "is then interrupted when a keypanel operator
  presses a key to talk to the person". *(Glossary text quoted from Clear-Com's Solution Finder wiki as indexed by
  search; the page's body did not render to direct fetch — **confidence: medium**.)*

**RTS** (*Digital Matrix Intercom System* product documentation): the abbreviations are the domain's shorthand —
**P-P** (point-to-point), **PL** (party-line), **ISO**, **IFB**, **SL** (special list), with keys "configured to
enable or disable latching function" and assignable to any of these types. Talk and listen indicators light
independently per key, and a key may be talk-only, listen-only, or talk + auto-listen.

**Trilogy** (*Gemini Digital Intercom Implementation Guide* v3.4a, Control Panels / Panel Keys and Tallies): key
types are enumerated as "**Listen; Speak; Listen and Speak; IFB; or a Phone key type**". Tallies: "**Red lamp**
indicates when you are speaking", "**Yellow lamp** indicates when you are listening" — and the highest-value detail
in the whole survey: the yellow lamp will "flash at a medium rate if the audio is present and **the key is not
selected to listen to the source**". Keys are individually configurable as latching or momentary. IFB interrupt
sources have a configurable **interrupt priority**.

### 2.9 Air traffic control

- **Roles are the unit of configuration, and they are transferable.** Rohde & Schwarz R&S®VCS-4G: "a **role-specific
  configuration is loaded when a controller signs in at a CWP**. The GUI is then automatically customized according
  to the profile downloaded for the controller in question. **Roles can become active or phased out depending on the
  time of day or other operational criteria. Controlled handover and delegation of roles is supported.**"
  (R&S®VCS-4G product brochure.) CERTIUM VCS extends this to cross-centre continuity: on a centre outage "controllers
  from another center can immediately take over airspace control **by simply having new roles allocated to them**"
  (CERTIUM VCS product brochure).
- **Priority is a protocol field.** ED-137 (VoIP for ATM) carries a SIP **Priority** header whose values are drawn
  from a table by call type — emergency / urgent / normal / non-urgent — defaulting to non-urgent when absent.
  *(ED-137 is paywalled; described from derived sources — **confidence: low–medium**.)*
- **Vacant positions are handled by procedure, not by UI.** FAA Order JO 7210.3 governs this through para 2-2-4
  *Duty Familiarization and the Transfer of Position Responsibility* and para 2-2-6 *Sign In/Out and On/Off
  Procedures*, which require the relieving controller to make "a statement or otherwise indicating to the specialist
  being relieved that position responsibility has been assumed". Sectors are combined into staffed positions rather
  than left unattended. *(Paragraph subjects verified via citation in other official FAA documents; full text not
  read — **confidence: medium**.)*

---

## 3. Cross-cutting: vocabulary

The terminology evidence is the highest-value output of this research, because VoxLoop's open question —
*"a loop is variously one person, a group, or a role someone stands behind"* — is a question every one of these
systems has already answered the same way.

| System | Conference / channel | Person | Staffable position | Subscription construct |
|---|---|---|---|---|
| NASA MCC | **loop** (Flight Director loop, Air-to-Ground, Front-to-Back, conference loop) | controller | **console position** with a call sign (FLIGHT, MMACS, CAPCOM, EGIL, BOOSTER) | monitor/talk mode per channel; saved **pages** |
| NASA HOSC | **conference** / voice loop | user (individual user IDs) | — (not modelled) | keys on a **key frame matrix**, per-user login |
| ESA ESOC | **voice loop** | engineer on console | position with a loop **call sign** (OD, SOM, Ground Ops Manager) | — |
| DLR openvocs | **loop** (= an audio conference) | **user** | **role** | `state: recv / send / none` per loop |
| Motorola | **resource** (channel / talkgroup); MSEL and Patch groups | radio unit / dispatcher | console position | **select / unselect** |
| Cisco IPICS | **channel**, **talk group**, **virtual talk group (VTG)** | user / **participant** | dispatcher / operator role; **ops view** scoping | channel assignment, latch permission |
| Speakerbus ARIA | **hoot** / **speaker channel** | trader | — (not modelled) | key, group-talk key |
| Clear-Com / RTS | **partyline (PL)**, **point-to-point (P-P)**, **IFB**, **ISO**, **SL** | — | **port** / panel *is* the identity | talk key + listen key |
| Trilogy | conference; key types Listen / Speak / Listen and Speak / IFB / Phone | — | panel | key + tally |
| ATC (R&S) | frequency / sector; telephone line | controller | **ATC role**, bound to a CWP at sign-in | direct / indirect access keys |

Three findings fall straight out of this table.

**A. "Loop" never means a person, and never means a role.** In every system that uses the word, a loop is an audio
conference — a many-to-many bus. The thing that has a name like "FIDO" or "OD" or "Ground Station Kourou" is a
*position*, and positions are separate objects. NASA's protocol makes the distinction audible: "Flight, MMACS"
addresses two positions, on a loop that is named after neither of them.

**B. Where a system does not model roles, it substitutes hardware identity.** Broadcast intercom has no user
concept at all — the *port* is the identity, and whoever is wearing the headset is that port. This works only
because the panel is physically bolted to a position. It is not available to a software product and is the single
biggest reason broadcast intercom's model does not transplant.

**C. openvocs and R&S independently converged on the same answer VoxLoop needs.** A user has roles; a role has
loops and a layout; you sign in *as a role*; the role determines what you may hear and say. This is exactly the
brief's "an operator may only ever allow one entity to be its loop and we should be able to see who is currently
taking on the role" — restated as a data model.

**Recommendation.** Adopt three terms and never overload them:

- **User** — an account. A person.
- **Role** — a staffable position (`Flight Director`, `Support Engineer`, `Ground Station Kourou`). Carries the
  permissions and the layout. May be constrained to one occupant or allow many.
- **Loop** — an audio conference. Joinable, monitorable, speakable-to. Never a person.

"A loop that is one person" then stops being a special case: it is a *role* with a single-occupant constraint,
which may or may not have a dedicated loop attached. This dissolves the terminology problem rather than solving it.

---

## 4. Cross-cutting: the six focus questions

### 4.1 Emission modes in practice

| Mode | Where it appears | Where it is deliberately absent |
|---|---|---|
| **Momentary PTT** | Universal. NASA hand unit / foot pedal; openvocs PTT bar; Motorola Instant Transmit ("press and hold"); Clear-Com non-latching key; RTS non-latch; Trilogy momentary; ARIA "Push To Talk"; IPICS PTT | Nowhere |
| **Latched** | Commercial systems universally: Clear-Com, RTS, Trilogy (per-key configurable), ARIA "Push To Latch", IPICS `Allow Latch` (a per-user, per-channel permission) | **Mission control.** Neither the NASA console described by Patterson nor openvocs offers a latch. Both require a key press every transmission. |
| **Open mic / always-on** | Trading floors only — "open hoot"/"shout down", "a permanent open circuit … Any user can speak at any time" | Mission control, dispatch, ATC, broadcast talkback |
| **Priority override / forced injection** | Motorola emergency; ARIA "barge in"; Clear-Com ISO/IFB; ED-137 priority header; Trilogy IFB interrupt priority | NASA — it is a *verbal* convention ("Break! Break!"), not a machine feature |

The finding that matters: **arming and keying are separate everywhere, and the two-stage structure is what makes
multi-channel emission safe.** NASA: set a channel to talk mode, then press the hand unit. openvocs: set the loop
tile green, then press PTT. Motorola: select resources, then General Transmit. ARIA: assign channels to the group
talk key, then press it. The arm step is deliberate, sticky and visible; the key step is momentary and cheap.
Collapsing them into one gesture is how you get an operator who does not know where their voice went.

The wayfinder's decision — momentary plus latch, with latch subsuming open mic — is well supported. Latch is
industry-standard everywhere outside the mission-control lineage, and the deliberate absence of open mic in every
safety-critical domain is a strong endorsement of dropping it.

The wayfinder's departure worth noting: **NASA and openvocs both enforce one-talk-loop-at-a-time.** Two independent
mission-control implementations arrived at that constraint. VoxLoop intends multi-destination emission. That is
defensible — dispatch and trading do it routinely — but it is the riskiest divergence in the design, and the
systems that permit it pay for it explicitly: ARIA prints a warning in the manual, caps group talk at eight
channels, and ships Solo mode; Clear-Com ships Local Exclusive. If VoxLoop allows multi-destination emission it
must ship the corresponding safety: an always-visible, unambiguous rendering of the complete destination set at the
moment of keying, and a one-action collapse to a single destination.

### 4.2 Monitoring vs talking on the console

Every system renders these as **separate, simultaneously visible states per destination**, and every one uses
colour rather than text.

| System | Not engaged | Monitoring | Armed to talk | Transmitting now |
|---|---|---|---|---|
| openvocs | grey tile | **blue** tile | **green** tile | PTT bar green + live waveform + talker name under tile + white frame on the loop |
| Clear-Com | unlit key | **bright green** (listen key) | — | **bright red** (talk / talk-with-listen key) |
| Trilogy | unlit | **yellow** lamp | — | **red** lamp |
| Motorola | plain resource tile | routed to unselect speaker | white background + green border (selected) | Instant Transmit button turns red; red lightning bolt on all resources in an MSEL group |
| ARIA Hoot | — | VAD indicators | — | talk state icon: "microphone is open" |
| IPICS | — | — | listen-only shown as **diagonal lines** over the PTT area | latch button green, PTT area yellow |

Two conventions are effectively universal and should be treated as free wins:

- **Red means your microphone is live; green means you are receiving.** Clear-Com, Trilogy and Motorola agree.
  (openvocs inverts green to mean "armed" — the one dissenter, and arguably the weaker choice, since it puts green
  on the dangerous state.)
- **Permission is drawn on the control itself, before the action is attempted.** openvocs draws a headphone icon
  for monitor-only and headphone+mic for talk-capable. ARIA draws a Listen-Only State icon. IPICS hatches the PTT
  area with diagonal lines. This directly serves VoxLoop's "listen-only is a permission state" constraint: the tile
  should say what you may do, not just what you are doing.

That gives **three independent bits per loop** that must be separately legible: *may I talk here*, *am I hearing
this*, *is my microphone open to this*. VoxLoop's UI needs all three; two is not enough.

### 4.3 How operators track who can hear them

This is the weakest area across the entire survey — nobody does it well, which is both a warning and an opportunity.

What exists:

- **Talker identity, per loop.** openvocs: "the name/role of the user that has an open microphone will appear
  under the tile name". IPICS: "the PTT area may show the talker ID". Motorola: an Activity Log of recent call
  activity with emergency entries highlighted red.
- **Peer transmit state.** Motorola broadcasts transmit status to *every* console monitoring that resource: "A
  parallel console position that attempts to transmit also causes these messages to be sent to all console
  positions that are assigned to or monitor that same resource." So you can see that someone else is keyed on a
  resource you share.
- **Occupancy count.** openvocs alone: "Icon with number => shows the number of active users in the loop." This is
  the only *roster-ish* affordance found anywhere.
- **Activity on loops you are not monitoring.** Trilogy alone: the listen tally "will flash at a medium rate if the
  audio is present and the key is not selected to listen to the source."

What does not exist anywhere: a list of *who is currently listening to me*. Mission control substitutes a behaviour
for it — Patterson's controllers monitor a colleague's Front-to-Back loop to infer whether they are busy, because
"controllers can listen in on communications without disrupting or even alerting the participants". Presence is
inferred from audio, not displayed.

**This is VoxLoop's clearest differentiation opportunity**, and it maps exactly onto the brief's requirement that
users "need to be able to easily see who they are emitting to and who is listening to them … so that they can very
quickly and easily request someone actually listen to them". Nothing surveyed does this. Trilogy's flashing tally
is the cheapest high-value idea in the whole survey and is trivial in software.

### 4.4 Per-source volume — does it exist, and do operators use it?

**Yes, universally, and yes — with direct observational evidence that it is a primary attention-management tool,
not a nicety.**

| System | Mechanism |
|---|---|
| NASA MCC | Per-loop volume controls, actively used to set relative loudness (Patterson et al., §4.1, §7) |
| NASA HOSC | "individual volume control" among per-user saved settings; "Gain and attenuation controls" in the requirements table |
| openvocs | Per-loop slider, 0–100, default 50 |
| Motorola | Per-resource volume slider; `All Mute` for unselected audio; emergency forces the resource to maximum |
| Clear-Com | Per-listen-key level via rotary control, plus **Reset Listen Levels** to restore all to default |
| ARIA Hoot | Per-channel volume slider, per-channel mute, master mute, **Solo mode** |

The observational evidence is unusually strong. Patterson et al. found controllers reprioritising *by volume* as
the operational situation changed, and identify it as a reason the medium works at all:

> "The ability to dynamically adjust volume levels on the different loops enables differentiation of the loop
> communications and directing attention to particular threads of activity." (§7)

But the survey also reveals the **failure mode**, and two vendors have shipped a fix for it. A loop turned down
during a quiet period and forgotten is a silent, invisible failure — the operator believes they are monitoring
something they cannot hear. Clear-Com's answer is a global "Reset Listen Levels" to maximum. Motorola's is to
override the operator's setting entirely when it matters: on an emergency, "The volume of the resource is
automatically set to maximum", cleared only by an explicit `Knockdown`.

**Implication for VoxLoop.** Per-loop volume: yes, clearly. But it must come with (a) a persistent visual
indication that a loop is attenuated or muted — not just a slider position buried behind a click, and (b) a
one-action reset-all, and (c) a rule that priority/emergency audio overrides the recipient's own attenuation.
Given the wayfinder's "state shown must always be factual", an attenuated loop that *looks* subscribed is exactly
the misrepresentation that constraint forbids.

### 4.5 Priority, override and forced injection

Two distinct mechanisms exist and they are not interchangeable:

**Additive priority** — the message still arrives as one more stream, but it is made louder and more salient.
Motorola's emergency (max volume, flashing red border, distinct tone, cleared at all positions monitoring the
resource). ED-137's SIP Priority header. Motorola's "Primary" resource marking.

**Interruptive priority** — the recipient's *other* audio is cut or ducked so the message lands alone. This is the
broadcast tradition and it is the more powerful primitive:

- **IFB** (Interruptible Fold Back): the recipient's normal programme feed "is then interrupted when a keypanel
  operator presses a key to talk to the person". Trilogy exposes `IFB cut/dim action`, `IFB dim level` and
  `IFB interrupt priority` as separate configuration — so an override may either *cut* or merely *duck* the
  competing audio, at a configurable amount, with a configurable precedence.
- **ISO**: "allows a panel operator to call a destination and interrupt all of that destination's other audio paths
  and establish a private conversation."
- **Barge in** (ARIA): join a call that is busy elsewhere.

NASA has neither — it has "Break! Break!", a verbal convention, "rarely used". This is a genuine finding rather
than an oversight: in a domain where every transmission is recorded and reviewed, a *social* override leaves an
auditable trail and cannot be abused silently, whereas a machine override can.

**Implication for VoxLoop.** The wayfinder lists "Admin override and injection" as unspecified. The evidence says:

1. Model it as **interruptive, with a dim option** — cut or duck the recipient's other loops, don't just add a
   stream. `IFB cut/dim action` + `IFB dim level` is the shape.
2. Give it a **numeric precedence**, not a boolean. Trilogy and ED-137 both do; Motorola's Enhanced Patch calls
   "have higher priority than ordinary Patch Group calls".
3. Configure it **per relationship in advance**, not improvised in the moment — it is an IFB *route*, not a button
   that means "shout".
4. Make it **loud in the audit trail**. NASA's verbal convention is socially self-limiting; a machine override is
   not, and ESA records everything on the loop for exactly this reason.

### 4.6 Vacant positions — nobody has solved this

The brief's requirement — *"if you're speaking to the support engineers loop, you need to know whether anyone's
actually behind it or not"* — is, on this evidence, **genuinely novel**. Every system surveyed either sidesteps it
or handles it by procedure:

| Domain | How "nobody is there" is handled |
|---|---|
| Broadcast intercom (Clear-Com, RTS, Trilogy) | Not modelled at all. A port always exists; whether a human is wearing the headset is unknowable to the system. |
| Motorola dispatch | Only *resource* availability is shown ("When a Primary Resource is not operational, it is marked with the following icon"). Radio infrastructure health, not human staffing. |
| NASA / ESA | Procedure. Formal roll call — "each engineer on console confirms to the Flight Operations Director their personal readiness" — plus shift handover. The position is staffed by definition; the process ensures it. |
| ATC | Procedure, and *combination*. FAA JO 7210.3 governs sign in/out and transfer of position responsibility; sectors are combined into staffed positions rather than left vacant. |
| R&S VCS-4G | The closest to a machine answer: "Roles can become **active or phased out** depending on the time of day or other operational criteria. Controlled **handover and delegation** of roles is supported." |
| openvocs | The only per-loop occupancy indicator found: "Icon with number => shows the number of active users in the loop." |

Synthesising these gives VoxLoop a design that is better than all of them and is not speculative:

- A **role** is either occupied or vacant, and this is a first-class, visible state (openvocs' count, generalised).
- A vacant role's loop **does not silently absorb speech** — the brief's instinct that it "goes auto-offline if no
  one's currently taking on the role" is right, and matches ATC's refusal to leave a position unattended.
- A vacant role can be **delegated or combined** into another staffed role (R&S; FAA position combining), rather
  than merely showing an error.
- **Sign-in/sign-out is an explicit act with a handover semantic**, not an implicit consequence of a websocket
  connecting (FAA 2-2-4 / 2-2-6, ESA roll call). This also gives the brief's "off console" action a natural home:
  it is a role occupancy state, not a user status.

---

## 5. What VoxLoop should borrow

Ranked by expected reduction in operator cognitive load per unit of build cost.

### 5.1 The three-term vocabulary: user / role / loop
See [§3](#3-cross-cutting-vocabulary). This is the highest-value finding in the research and it costs nothing to
adopt. It should land in `CONTEXT.md` before any further design work, because the current ambiguity will
otherwise leak into the permission model, the UI and the API.

### 5.2 Two-stage emission: arm, then key
Universal. Arming a loop for talk is sticky, deliberate and visible; keying is momentary. openvocs models this
cleanly as two separate API events (`switch_state` with `send`, and `switch_ptt`), and VoxLoop should too — the
"which loops am I pointed at" state must be independently queryable from "am I transmitting right now".

### 5.3 Three legible bits per loop, colour-coded
*May I talk here* / *am I hearing this* / *is my mic open to this*. Draw permission on the tile (openvocs'
headphone vs headphone+mic; ARIA's Listen-Only icon; IPICS' diagonal hatching). Keep red for a live microphone.

### 5.4 Per-loop volume — with a reset and a priority override
Per-loop gain is used constantly and is the primary attention tool. Ship it with a persistent "this loop is
attenuated" indicator, a one-action reset-all (Clear-Com), and priority audio that ignores the recipient's
attenuation (Motorola). Without those it becomes a silent misrepresentation of what the operator can hear.

### 5.5 Talker identity by role, shown on the loop where it is happening
openvocs shows the speaker's name/role under the loop tile. Combined with the role vocabulary, this means an
operator sees "FLIGHT is talking on Coordination" rather than "ed.hutchinson is talking".

### 5.6 Activity indication for loops you are *not* monitoring
Trilogy's flashing listen tally. Cheap in software, and it directly enables the highest-value behaviour Patterson
et al. observed: judging whether someone is interruptible before interrupting them.

### 5.7 An exclusive / solo escape hatch
Clear-Com's Local Exclusive and ARIA's Solo mode. One action that collapses to a single destination — both for
talking (make sure my voice goes to exactly one place) and listening (mute everything except this). This is the
required counterweight to multi-destination emission.

### 5.8 Pre-provision loops; do not make operators create them under load
Patterson et al. make this argument explicitly and it is the strongest guidance available for VoxLoop's open
"breakout / isolated channel" question:

> "Another contrasting design concept would be to allow controllers to create any loops that they might want at
> any point in time. This extreme flexibility would create unnecessary burdens for the practitioners. It would
> force the controllers to figure out for themselves all the people that they might want to talk to and negotiate
> who should be on each of the loops. The need for loops that are used infrequently, such as conference loops,
> might not be recognized until a problem occurs. It would then be too difficult during the high-tempo response
> period to create the loops … In addition, loops would be created in ways that would be idiosyncratic to the
> particular teams rather than standardized. Without standardization, controllers would have to memorize the setup
> of specific loops **in order to know who listens to them**." (§7)

Their solution is dedicated, standing, named conference loops that "are continuously monitored but lie unused
until a situation arises". Applied to VoxLoop: breakout should be **"move to Conference 3"**, a pre-existing
named loop with known membership semantics, not **"create a channel and invite people"**. The former is one
action and zero decisions; the latter is a configuration task during a crisis.

### 5.9 Role-based layouts loaded at sign-in
openvocs loads the loop set *and the tile layout* for the selected role; R&S loads a role-specific GUI profile at
CWP sign-in and supports roles that activate or phase out by time of day. This answers the wayfinder's open
"default configurations and profiles" item almost completely, including the admin-enforced-at-certain-times part.

### 5.10 Interrupt semantics for override, with cut/dim and numeric precedence
See [§4.5](#45-priority-override-and-forced-injection).

### 5.11 Stable spatial position for loops
NASA's saved "pages", HOSC's "loop location on the key frame matrix", openvocs' per-role grid layout. Operators
navigate by muscle memory. A grid whose tiles move — because a loop went quiet, or a list re-sorted by activity —
destroys that. **Never auto-reorder the operator's grid.** This is a cheap decision that is expensive to reverse.

### 5.12 Recording is table stakes, not a later nicety
ESA: "all communications are recorded". NASA EVoDS provided "record and playback capability". openvocs ships a
recorder service and a `get_recordings` API. The wayfinder's constraint that the architecture must leave a
decoded-audio seam is correct and is corroborated by every operational deployment surveyed.

---

## 6. Do not copy these

Legacy hardware artefacts, ranked by how tempting they are to copy by mistake.

### 6.1 Select vs unselect speaker
Motorola's central audio concept — selected resource to the headset/select speaker, everything else to a separate
unselect speaker — exists because the console had **two physical loudspeakers**. It is a hardware routing matrix,
not a user-experience idea. VoxLoop has one output. The useful part (a notion of *focus* versus *background*) is
fully served by per-loop gain plus a solo action. **Do not build two audio buses.**

### 6.2 The squelch/monitor dance before transmitting
Motorola instructs the operator: "To avoid overriding another transmission in progress: Select and hold the
Monitor button … This momentarily disables the squelch on the resource so the operator can listen to transmissions
on other private lines. If no one else is speaking, perform a transmission." This is an analogue half-duplex radio
problem. In a software mixer, simultaneous talkers mix; there is no doubling penalty and no need for a
listen-before-transmit ritual. The brief already says so — "more than one entity that you're subscribed to can
speak at once and that's fine".

### 6.3 Fixed key counts, lever panels, pages of 8/16/32
Trilogy ships 8- to 32-lever panels; RTS panels have fixed key counts; NASA's "pages" and HOSC's "key frame
matrix" exist partly because a physical panel has a fixed number of buttons. The *reason* for stable positions is
real ([§5.11](#511-stable-spatial-position-for-loops)) but the fixed capacity is not. Do not design around
"a page holds N loops".

### 6.4 Open mic / hot mic / "open hoot"
Present only on trading floors. Absent from mission control, dispatch, ATC and broadcast talkback. The wayfinder's
decision to collapse open mic into latch is well supported by the evidence — latch gives the same hands-free
capability while keeping an explicit, revocable, visible act of opening the microphone.

### 6.5 The radio and telephony estate
Sidetone, gain/attenuation stages, non-blocking switch fabrics, T1/DS0, MFC-R2/E&M/FXO interfaces, trunking and
trunk masters, tone signalling (Motorola's numbered Alert Tones 1–15), DTMF generation, ring-down (ARD/MRD),
carrier-operated relay detection, patching dissimilar frequencies, GPI/GPO relay outputs, door locks in the GUI.
All of these are artefacts of interfacing to physical radio and telephone plant. VoxLoop interfaces to neither.

### 6.6 Instant Recall Recorder as a console widget
Motorola's EIRR, and its intricate rules about which audio it does and does not capture (it records selected
resources but not unselected ones, not outbound patch audio, and so on). This complexity exists because recording
happened at the console. Server-side recording — already the wayfinder's plan — makes the whole category disappear.

### 6.7 "Loop" as a synonym for a person or a role
Named explicitly because it is the artefact VoxLoop is currently at risk of inventing rather than inheriting. No
surveyed system does this. See [§3](#3-cross-cutting-vocabulary).

### 6.8 Deliberate divergence, flagged: one-talk-loop-at-a-time
Both mission-control systems enforce it (NASA: "Only one channel at a time can be set to the talk mode";
openvocs: "only one for talking mode … the former voice loop will switch back to monitoring automatically").
VoxLoop intends to allow multi-destination emission. This is a *conscious* divergence with precedent in dispatch
and trading — but it is the riskiest one in the design, and the systems that permit it all ship compensating
safety (ARIA's manual warning, its eight-channel cap and Solo mode; Clear-Com's Local Exclusive). If VoxLoop keeps
multi-destination emission, the compensations are not optional.

---

## 7. Open questions the evidence does not settle

- **Whether multi-destination emission is actually worth its cost.** Two mission-control systems refused it. The
  evidence establishes the risk clearly but cannot tell us whether VoxLoop's users need it. This is a
  prototype/grilling question, not a research one.
- **How large a loop set an operator can hold.** NASA controllers monitor ~4 of 164 available loops; ARIA caps
  group talk at 8; dispatch consoles carry dozens of resources. No source gives a defensible upper bound for
  *tiles on screen*.
- **What a "listening roster" costs in practice.** No surveyed system displays who is listening, so there is no
  evidence about whether operators find it useful or merely noisy. VoxLoop would be first.
- **Cisco IPICS detail.** cisco.com refused all direct fetches; §2.6 rests on search-indexed excerpts of the
  official guides. If IPICS specifics become load-bearing for a decision, the guides should be retrieved by other
  means and the section re-verified.
- **ED-137 priority semantics.** Paywalled; §2.9's priority claim is derived. If VoxLoop's override model ends up
  wanting a standards-aligned precedence scheme, buy the standard.
- **ESA/ESOC console mechanics.** Public ESA material confirms vocabulary and practice but gives nothing about the
  console itself. Ed's own operational experience is a better source here than anything published, and should be
  captured directly rather than researched.

---

## 8. Sources

**Space operations**

- Patterson, E.S., Watts-Perotti, J., & Woods, D.D. (1999). *Voice Loops as Coordination Aids in Space Shuttle
  Mission Control.* Computer Supported Cooperative Work. NASA Grant NAG9-390.
  <https://www.interruptions.net/literature/Patterson-CSCW-JCC99.pdf> (also NTRS 20040088098)
- Pirani, J.L. & Calvelage, S.G. *Voice over Internet Protocol (VoIP) in a Control Center Environment.*
  NASA Marshall Space Flight Center. NTRS 20100020881.
  <https://ntrs.nasa.gov/api/citations/20100020881/downloads/20100020881.pdf>
- **openvocs** (DLR German Space Operations Center), official repository, README, client API and
  *User Guide v1.3 (DRAFT)*. Apache-2.0. <https://github.com/openvocs/openvocs> · <https://openvocs.org/>
- DLR. *GSOC publishes open source software platform "openvocs" for communication in mission control* (2025).
  <https://www.dlr.de/en/rb/latest/news/2025/gsoc-publishes-open-source-software-platform-openvocs-for-communication-in-mission-control>
- Töpfer, M., Sonnenberg, A. & Kozlowski, R.A. (2016). *OpenSource based Voice Communication for Mission Control.*
  SpaceOps 2016, AIAA 2016-2437. <https://elib.dlr.de/108827/>
- ESA. *Network Operations Centre.*
  <https://www.esa.int/Enabling_Support/Operations/ESA_Ground_Stations/Network_Operations_Centre>
- ESA. *Countdown* (ESOC). <https://www.esa.int/About_Us/ESOC/Countdown>
- ESA blogs. *ESOC readies for Sentinel-5P* (2017).
  <https://blogs.esa.int/eolaunches/2017/09/26/esoc-readies-for-sentinel-5p/>

**Dispatch and control-room intercom**

- Motorola Solutions. *MCC 7000 Series Dispatch Consoles: Elite Dispatch End User Toolkit*, ACS7173601.
- Motorola Solutions. *MCC 7500 IP Dispatch Console* data sheet.
  <https://www.motorolasolutions.com/content/dam/msi/docs/business/products/dispatch/dispatch_consoles/mcc_7500/_documents/static_files/mcc_7500_ip_dispatch_console_data_sheet.pdf>
- Cisco. *Cisco IPICS Server Administration Guide* — Performing Cisco IPICS Dispatcher Tasks; Configuring and
  Managing Operational Views; *Cisco IPICS Dispatch Console User Guide*.
  <https://www.cisco.com/c/en/us/td/docs/interoperability_systems/c_ipics/4102/administration/ag4102_print/agdisp.html>
  *(HTTP 403 to direct fetch; quoted via search index — see confidence ledger.)*
- US Patent 9,641,988 *Land mobile radio dispatch console* (and related grants) — corroborates the
  select/unselect + per-line volume + line PTT console architecture.

**Trading floor**

- Speakerbus. *ARIA Hoot User Guide*, rev 5, 3.300.2.0.
  <https://www.speakerbus.com/hubfs/Support%20Hub%20Detailed%20Guide/ARIA%20Hoot%20User%20Guide.pdf>
- Speakerbus. *Trader Voice 101: A Beginner's Guide to the Basics.*
  <https://www.speakerbus.com/blog/trader-voice-101-a-beginners-guide-to-the-basics>

**Broadcast talkback**

- Clear-Com. *i-Series Panels User Guide*, 399G207A (Eclipse HX v9.0).
  <https://clearcom.com/DownloadCenter/manuals/EclipseHX_v9.0/i-Series_Panels_User_Guide-399G207A.pdf>
- Clear-Com. *Glossary for Party Line Intercom*, Solution Finder.
  <https://clear-com.atlassian.net/wiki/spaces/SF/pages/322404618/Glossary+for+Party+Line+Intercom>
  *(body did not render to direct fetch; quoted via search index.)*
- RTS / Telex. *RTS Digital Matrix Intercom System* product documentation.
  <http://pdf.textfiles.com/manuals/STARINMANUALS/Telex%20Intercom/Archive/RTS%20Digital%20Matrix%20Intercom%20System.pdf>
- Trilogy Communications. *Gemini Digital Intercom Implementation Guide*, issue 3.4a.
  <https://clearcom.com/DownloadCenter/manuals/Trilogy/Trilogy_Gemini_UserGuide-70090620v3.4a.pdf>

**Air traffic control**

- Rohde & Schwarz. *R&S®VCS-4G IP-Based Voice Communications System* product brochure, 5214.5010.12.
- Rohde & Schwarz. *CERTIUM VCS* product brochure, 5214.5010.12 v1000.
- EUROCAE ED-137, *Interoperability Standards for VoIP ATM Components* (Vols 1–5). Paywalled.
- FAA Order JO 7210.3, *Facility Operation and Administration*, paras 2-2-4 (Duty Familiarization and the Transfer
  of Position Responsibility) and 2-2-6 (Sign In/Out and On/Off Procedures).
  <https://www.faa.gov/regulations_policies/orders_notices/index.cfm/go/document.current/documentNumber/7210.3>
