use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::Duration;

use figment::Figment;
use figment::providers::{Env, Format, Toml};
use serde::Deserialize;

/// Everything needed to reach the store, read from one file at startup and never re-read.
///
/// Configuration splits on auditability ([ADR-0040]). Anything an administrator changes
/// through the console lives in the store, because a file edit cannot be audited; anything
/// needed to *reach* the store lives here. A certificate change is therefore a restart, and
/// a restart ends every session — which is an argument for a long-lived certificate rather
/// than anything short-dated.
///
/// [ADR-0040]: ../../../docs/adr/0040-one-binary-one-unit-four-moving-parts.md
#[derive(Debug, Deserialize)]
pub(crate) struct Deployment {
    #[serde(default)]
    pub(crate) listen: Listen,
    pub(crate) tls: Tls,
    pub(crate) media: Media,
    pub(crate) store: StoreFile,
    #[serde(default)]
    pub(crate) connection: Ladder,
    #[serde(default)]
    pub(crate) log: Log,
}

/// One inbound TCP port carries HTTPS and the signalling WebSocket together.
#[derive(Debug, Deserialize)]
pub(crate) struct Listen {
    pub(crate) address: SocketAddr,
}

/// TLS terminates in this binary, so it holds the certificate itself ([ADR-0040]).
///
/// [ADR-0040]: ../../../docs/adr/0040-one-binary-one-unit-four-moving-parts.md
#[derive(Debug, Deserialize)]
pub(crate) struct Tls {
    pub(crate) certificate: PathBuf,
    pub(crate) private_key: PathBuf,
}

/// Where the audio goes: one port, and the address to put in an ICE candidate.
///
/// **`announced_address` has no default and never will.** Everything else in this file falls
/// back to something a box can run on; this cannot, because the wrong value here is the one
/// failure that looks like success — the console signs in, the loops render, the seats fill,
/// and no audio ever arrives, because every candidate VoxLoop offered named an address the
/// client cannot reach. A deployment that has not said where it is does not start.
///
/// There is **no TURN server** ([ADR-0006]): the box sits at a fixed address inside the
/// customer's network with remote users already on the VPN, so host candidates are expected
/// to work. That is a revisitable assumption rather than a finding.
///
/// [ADR-0006]: ../../../docs/adr/0006-mediasoup-carries-the-audio.md
#[derive(Debug, Deserialize)]
pub(crate) struct Media {
    /// The address a client dials, which is the one that goes into every ICE candidate.
    ///
    /// A hostname is accepted as well as an address, because mediasoup accepts one.
    pub(crate) announced_address: String,
    /// What the worker binds. Optional; every interface, which is what a box with one
    /// address and a firewall in front of it wants.
    #[serde(default = "Media::every_interface")]
    pub(crate) listen_address: IpAddr,
    /// **One** port, carrying UDP primarily and ICE-TCP where UDP is blocked ([ADR-0006]).
    ///
    /// One number rather than an ephemeral range, on both protocols rather than two, because
    /// the firewall conversation is a real cost of deploying this and a range is a much
    /// worse one.
    ///
    /// [ADR-0006]: ../../../docs/adr/0006-mediasoup-carries-the-audio.md
    #[serde(default = "Media::the_usual_port")]
    pub(crate) port: u16,
}

impl Media {
    fn every_interface() -> IpAddr {
        IpAddr::from([0, 0, 0, 0])
    }

    /// Nothing depends on the number; it is out of the ephemeral range, out of the way of
    /// anything registered, and the same on every deployment that does not say otherwise so
    /// that a firewall rule is one number somebody can recognise.
    fn the_usual_port() -> u16 {
        44444
    }

    /// Media on loopback, on whatever port is free. What a test asks for.
    #[cfg(test)]
    pub(crate) fn on_loopback() -> Self {
        Self {
            announced_address: "127.0.0.1".to_owned(),
            listen_address: IpAddr::from([127, 0, 0, 1]),
            port: 0,
        }
    }
}

/// The four timers of the signalling channel's health ladder ([ADR-0018], v1 §7).
///
/// They are **startup settings with a hard ceiling**, and both halves are the decision. The
/// disconnect threshold is a safety parameter rather than a display preference — it sets how
/// long a network hiccup can silence the loudest voice in the room, and it has to be tuned
/// against the pilot's VPN. An unbounded knob lets a site set it to five minutes and
/// reintroduce exactly the hot mic the rule removes, so a value past the ceiling is refused
/// at startup rather than clamped: clamping would leave an administrator believing a number
/// that is not in force.
///
/// Every value is **in seconds**. A duration written as a table of seconds and nanoseconds is
/// what serde does with `Duration`, and a deployment file nobody can read by eye is a
/// deployment file somebody gets wrong.
///
/// [ADR-0018]: ../../../docs/adr/0018-no-signalling-channel-means-no-emission-path.md
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct Ladder {
    /// How often each end says it is still there.
    #[serde(default = "Ladder::the_default_heartbeat")]
    heartbeat: u64,
    /// How long a session goes unheard from before its state is no longer confirmed.
    ///
    /// The middle rung is what makes disabling push-to-talk safe rather than fragile: a
    /// single threshold would mean a VPN reroute mutes the Flight Director mid-sentence,
    /// trading a state-honesty problem for a worse availability one.
    #[serde(default = "Ladder::the_default_unconfirmed")]
    unconfirmed: u64,
    /// How long a **latched** emission survives once the state is unconfirmed.
    ///
    /// A latch's entire safety story is that the console will show it to you, and that story
    /// is void the moment the console cannot be trusted. The grace is what keeps a brief blip
    /// from cutting a genuine latched transmission; past it, the latch dies. It is the client
    /// that runs this one, because it is the end holding the microphone — which is why it is
    /// carried to the console on every heartbeat rather than kept here.
    #[serde(default = "Ladder::the_default_latch_grace")]
    latch_dropped: u64,
    /// How long a session goes unheard from before emission is withdrawn at both ends.
    #[serde(default = "Ladder::the_default_disconnected")]
    disconnected: u64,
}

impl Ladder {
    /// The numbers v1 §7 fixes, which are what a deployment that says nothing runs on.
    ///
    /// One per setting rather than one per value, even where two of them are the same number:
    /// serde wants a function per field, and a function named for the number it returns would
    /// be a name that stops being true the day one of them is tuned.
    fn the_default_heartbeat() -> u64 {
        2
    }

    fn the_default_unconfirmed() -> u64 {
        5
    }

    fn the_default_latch_grace() -> u64 {
        2
    }

    fn the_default_disconnected() -> u64 {
        12
    }

    /// Each setting's ceiling, which is the point past which it stops being the thing it was
    /// named for.
    ///
    /// A heartbeat slower than this cannot measure the rungs below it, and a latch held
    /// longer than this is held past any blip worth waiting out. The two thresholds are
    /// capped at half a minute, which is already generous for a VPN reroute and well short of
    /// the *five minutes* ADR-0018 names as the failure this bound exists to refuse.
    fn ceilings(self) -> [(&'static str, u64, u64); 4] {
        [
            ("heartbeat", self.heartbeat, 10),
            ("unconfirmed", self.unconfirmed, 30),
            ("latch_dropped", self.latch_dropped, 10),
            ("disconnected", self.disconnected, 30),
        ]
    }

    pub(crate) fn heartbeat(self) -> Duration {
        Duration::from_secs(self.heartbeat)
    }

    pub(crate) fn unconfirmed(self) -> Duration {
        Duration::from_secs(self.unconfirmed)
    }

    pub(crate) fn latch_dropped(self) -> Duration {
        Duration::from_secs(self.latch_dropped)
    }

    pub(crate) fn disconnected(self) -> Duration {
        Duration::from_secs(self.disconnected)
    }

    /// Whether these four numbers are a ladder, and one VoxLoop will run on.
    ///
    /// The ceilings are the rule ADR-0018 asks for. The ordering is the same rule arriving
    /// from the other side: **a rung nothing can reach is a rule that silently does not
    /// exist**, and neither of the two ways to write one is individually alarming to read.
    /// `unconfirmed` past `disconnected` takes away the band that makes withdrawing emission
    /// safe; a latch grace that runs past `disconnected` means a latched emission is never
    /// dropped for being unshowable and only ever for the general withdrawal, which is the
    /// rule ADR-0018 wrote this setting for not happening. Nothing is clamped, for the reason
    /// the type gives.
    fn check(self, path: &Path) -> Result<(), DeploymentError> {
        for (setting, said, ceiling) in self.ceilings() {
            if said == 0 {
                return Err(DeploymentError::NotALadder {
                    path: path.to_path_buf(),
                    detail: format!("connection.{setting} is zero, which is not a timer"),
                });
            }
            if said > ceiling {
                return Err(DeploymentError::PastTheCeiling {
                    path: path.to_path_buf(),
                    setting,
                    said,
                    ceiling,
                });
            }
        }

        if self.heartbeat >= self.unconfirmed || self.unconfirmed >= self.disconnected {
            return Err(DeploymentError::NotALadder {
                path: path.to_path_buf(),
                detail: format!(
                    "connection.heartbeat ({}s), connection.unconfirmed ({}s) and \
                     connection.disconnected ({}s) have to climb in that order",
                    self.heartbeat, self.unconfirmed, self.disconnected
                ),
            });
        }

        if self.unconfirmed + self.latch_dropped >= self.disconnected {
            return Err(DeploymentError::NotALadder {
                path: path.to_path_buf(),
                detail: format!(
                    "connection.latch_dropped ({}s) leaves a latch standing {}s past the \
                     last heartbeat, at or past connection.disconnected ({}s), so a latched \
                     emission would never be dropped for being unshowable",
                    self.latch_dropped,
                    self.unconfirmed + self.latch_dropped,
                    self.disconnected
                ),
            });
        }

        Ok(())
    }
}

impl Default for Ladder {
    fn default() -> Self {
        Self {
            heartbeat: Self::the_default_heartbeat(),
            unconfirmed: Self::the_default_unconfirmed(),
            latch_dropped: Self::the_default_latch_grace(),
            disconnected: Self::the_default_disconnected(),
        }
    }
}

/// Where the one SQLite file lives.
#[derive(Debug, Deserialize)]
pub(crate) struct StoreFile {
    pub(crate) path: PathBuf,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Log {
    pub(crate) level: String,
}

impl Default for Listen {
    fn default() -> Self {
        Self {
            address: SocketAddr::from(([0, 0, 0, 0], 8443)),
        }
    }
}

impl Default for Log {
    fn default() -> Self {
        Self {
            level: "info".to_owned(),
        }
    }
}

/// What can go wrong reading the deployment file.
#[derive(Debug, thiserror::Error)]
pub(crate) enum DeploymentError {
    #[error("no deployment file at {path}")]
    Missing { path: PathBuf },

    #[error("the deployment file at {path} could not be read: {detail}")]
    Unreadable { path: PathBuf, detail: String },

    #[error(
        "the deployment file at {path} sets connection.{setting} to {said}s, past the {ceiling}s \
         VoxLoop accepts: a longer one is how long a network hiccup may leave a microphone \
         open that nobody can be told about"
    )]
    PastTheCeiling {
        path: PathBuf,
        setting: &'static str,
        said: u64,
        ceiling: u64,
    },

    #[error("the deployment file at {path} does not describe a ladder: {detail}")]
    NotALadder { path: PathBuf, detail: String },
}

impl Deployment {
    /// Read the deployment file, letting `VOXLOOP_`-prefixed environment variables override
    /// what is in it. `VOXLOOP_LISTEN__ADDRESS` overrides `address` under `[listen]`.
    ///
    /// This runs once, at startup. Nothing re-reads it, so every value here is fixed for the
    /// life of the process.
    pub(crate) fn load(path: &Path) -> Result<Self, DeploymentError> {
        if !path.exists() {
            return Err(DeploymentError::Missing {
                path: path.to_path_buf(),
            });
        }

        let deployment: Self = Figment::new()
            .merge(Toml::file(path))
            .merge(Env::prefixed("VOXLOOP_").split("__"))
            .extract()
            .map_err(|error| DeploymentError::Unreadable {
                path: path.to_path_buf(),
                detail: error.to_string(),
            })?;

        // The one section that is refused for what it says rather than for how it is
        // written. Everything else here is a path or an address, where *unreadable* is the
        // whole of what can be wrong with it.
        deployment.connection.check(path)?;

        Ok(deployment)
    }
}
#[cfg(test)]
// `figment::Jail` closures return `figment::Error`, which is a large type of figment's own.
#[allow(clippy::result_large_err)]
mod tests {
    use super::*;
    use std::net::{IpAddr, SocketAddr};

    const A_WHOLE_FILE: &str = r#"
        [listen]
        address = "10.0.0.4:8443"

        [tls]
        certificate = "/etc/voxloop/tls/fullchain.pem"
        private_key = "/etc/voxloop/tls/privkey.pem"

        [media]
        announced_address = "10.0.0.4"
        listen_address = "10.0.0.4"
        port = 45000

        [store]
        path = "/var/lib/voxloop/voxloop.sqlite"

        [log]
        level = "warn"
    "#;

    #[test]
    fn reads_the_file_it_is_given() {
        figment::Jail::expect_with(|jail| {
            jail.create_file("voxloop.toml", A_WHOLE_FILE)?;

            let deployment = Deployment::load(Path::new("voxloop.toml")).expect("a deployment");

            assert_eq!(
                deployment.listen.address,
                "10.0.0.4:8443".parse::<SocketAddr>().unwrap()
            );
            assert_eq!(
                deployment.tls.certificate,
                PathBuf::from("/etc/voxloop/tls/fullchain.pem")
            );
            assert_eq!(
                deployment.tls.private_key,
                PathBuf::from("/etc/voxloop/tls/privkey.pem")
            );
            assert_eq!(
                deployment.store.path,
                PathBuf::from("/var/lib/voxloop/voxloop.sqlite")
            );
            assert_eq!(deployment.media.announced_address, "10.0.0.4");
            assert_eq!(
                deployment.media.listen_address,
                "10.0.0.4".parse::<IpAddr>().unwrap()
            );
            assert_eq!(deployment.media.port, 45000);
            assert_eq!(deployment.log.level, "warn");
            Ok(())
        });
    }

    #[test]
    fn lets_the_environment_override_the_file() {
        figment::Jail::expect_with(|jail| {
            jail.create_file("voxloop.toml", A_WHOLE_FILE)?;
            jail.set_env("VOXLOOP_LISTEN__ADDRESS", "127.0.0.1:9443");
            jail.set_env("VOXLOOP_LOG__LEVEL", "debug");

            let deployment = Deployment::load(Path::new("voxloop.toml")).expect("a deployment");

            assert_eq!(
                deployment.listen.address,
                "127.0.0.1:9443".parse::<SocketAddr>().unwrap()
            );
            assert_eq!(deployment.log.level, "debug");
            Ok(())
        });
    }

    #[test]
    fn falls_back_to_a_listen_address_and_a_log_level_but_never_to_a_certificate() {
        figment::Jail::expect_with(|jail| {
            jail.create_file(
                "voxloop.toml",
                r#"
                [tls]
                certificate = "/etc/voxloop/tls/fullchain.pem"
                private_key = "/etc/voxloop/tls/privkey.pem"

                [media]
                announced_address = "10.0.0.4"

                [store]
                path = "/var/lib/voxloop/voxloop.sqlite"
                "#,
            )?;

            let deployment = Deployment::load(Path::new("voxloop.toml")).expect("a deployment");

            assert_eq!(
                deployment.listen.address,
                "0.0.0.0:8443".parse::<SocketAddr>().unwrap()
            );
            assert_eq!(deployment.log.level, "info");
            // Every interface and one recognisable port, so a box with one address and a
            // firewall in front of it needs to say only where it is.
            assert_eq!(
                deployment.media.listen_address,
                "0.0.0.0".parse::<IpAddr>().unwrap()
            );
            assert_eq!(deployment.media.port, 44444);
            Ok(())
        });
    }

    /// The four numbers v1 §7 fixes. A deployment that says nothing about the ladder runs on
    /// them, which is what makes the section optional rather than a fifth thing every site
    /// has to get right.
    #[test]
    fn falls_back_to_the_ladder_v1_fixes() {
        figment::Jail::expect_with(|jail| {
            jail.create_file("voxloop.toml", A_WHOLE_FILE)?;

            let deployment = Deployment::load(Path::new("voxloop.toml")).expect("a deployment");

            assert_eq!(deployment.connection.heartbeat(), Duration::from_secs(2));
            assert_eq!(deployment.connection.unconfirmed(), Duration::from_secs(5));
            assert_eq!(
                deployment.connection.latch_dropped(),
                Duration::from_secs(2)
            );
            assert_eq!(
                deployment.connection.disconnected(),
                Duration::from_secs(12)
            );
            Ok(())
        });
    }

    #[test]
    fn takes_a_ladder_the_deployment_tuned_for_itself() {
        figment::Jail::expect_with(|jail| {
            jail.create_file(
                "voxloop.toml",
                &with_a_ladder("heartbeat = 3\nunconfirmed = 8\ndisconnected = 20"),
            )?;

            let deployment = Deployment::load(Path::new("voxloop.toml")).expect("a deployment");

            assert_eq!(deployment.connection.heartbeat(), Duration::from_secs(3));
            assert_eq!(deployment.connection.unconfirmed(), Duration::from_secs(8));
            assert_eq!(
                deployment.connection.disconnected(),
                Duration::from_secs(20)
            );
            // Untouched, and still the number v1 fixes: the section is four settings and a
            // file may say anything from one of them to all four.
            assert_eq!(
                deployment.connection.latch_dropped(),
                Duration::from_secs(2)
            );
            Ok(())
        });
    }

    /// **The disconnect threshold is a safety parameter, not a display preference**
    /// (ADR-0018). It is how long a network hiccup may leave a microphone open that nobody
    /// can be told about, so a site that asks for five minutes is refused rather than quietly
    /// given thirty seconds — an administrator believing a number that is not in force is the
    /// worse of the two failures.
    #[test]
    fn refuses_a_disconnect_threshold_past_the_ceiling() {
        figment::Jail::expect_with(|jail| {
            jail.create_file("voxloop.toml", &with_a_ladder("disconnected = 300"))?;

            let Err(refusal) = Deployment::load(Path::new("voxloop.toml")) else {
                panic!("expected a refusal to start");
            };

            assert!(
                matches!(
                    refusal,
                    DeploymentError::PastTheCeiling {
                        setting: "disconnected",
                        said: 300,
                        ..
                    }
                ),
                "expected a refusal naming the setting, got {refusal:?}"
            );
            Ok(())
        });
    }

    /// All four are bounded, not only the one the ADR argues about. A heartbeat that cannot
    /// measure the rungs below it and a latch held past any blip worth waiting out are the
    /// same failure reached by other doors.
    #[test]
    fn refuses_any_of_the_four_past_its_ceiling() {
        for (setting, said) in [
            ("heartbeat", 60),
            ("unconfirmed", 90),
            ("latch_dropped", 45),
            ("disconnected", 300),
        ] {
            figment::Jail::expect_with(|jail| {
                jail.create_file(
                    "voxloop.toml",
                    &with_a_ladder(&format!("{setting} = {said}")),
                )?;

                let refusal = Deployment::load(Path::new("voxloop.toml"))
                    .err()
                    .unwrap_or_else(|| panic!("expected {setting} = {said} to be refused"));

                assert!(
                    matches!(refusal, DeploymentError::PastTheCeiling { setting: named, .. } if named == setting),
                    "expected a refusal naming {setting}, got {refusal:?}"
                );
                Ok(())
            });
        }
    }

    /// A ladder whose middle rung is never reached is not a ladder. `unconfirmed` past
    /// `disconnected` would take away the band that makes withdrawing emission safe, without
    /// any number in the file being individually alarming.
    #[test]
    fn refuses_rungs_that_do_not_climb() {
        figment::Jail::expect_with(|jail| {
            jail.create_file(
                "voxloop.toml",
                &with_a_ladder("unconfirmed = 20\ndisconnected = 12"),
            )?;

            let Err(refusal) = Deployment::load(Path::new("voxloop.toml")) else {
                panic!("expected a refusal to start");
            };

            assert!(
                matches!(refusal, DeploymentError::NotALadder { .. }),
                "expected a refusal, got {refusal:?}"
            );
            Ok(())
        });
    }

    /// A timer of nothing is not a fast deployment, it is a deployment with no band at all —
    /// every session would be unconfirmed the instant it was heard from.
    #[test]
    fn refuses_a_timer_of_zero() {
        figment::Jail::expect_with(|jail| {
            jail.create_file("voxloop.toml", &with_a_ladder("heartbeat = 0"))?;

            let Err(refusal) = Deployment::load(Path::new("voxloop.toml")) else {
                panic!("expected a refusal to start");
            };

            assert!(
                matches!(refusal, DeploymentError::NotALadder { .. }),
                "expected a refusal, got {refusal:?}"
            );
            Ok(())
        });
    }

    /// **A rung nothing can reach is a rule that silently does not exist.** A latch grace
    /// running past the disconnect threshold means a latched emission is only ever dropped by
    /// the general withdrawal, and never for the reason ADR-0018 gave the setting — with no
    /// number in the file individually alarming to read.
    #[test]
    fn refuses_a_latch_grace_that_outlives_the_disconnect_threshold() {
        figment::Jail::expect_with(|jail| {
            jail.create_file("voxloop.toml", &with_a_ladder("latch_dropped = 10"))?;

            let Err(refusal) = Deployment::load(Path::new("voxloop.toml")) else {
                panic!("expected a refusal to start");
            };

            assert!(
                matches!(refusal, DeploymentError::NotALadder { .. }),
                "expected a refusal, got {refusal:?}"
            );
            Ok(())
        });
    }

    /// A whole file, with a `[connection]` section saying whatever the test is about.
    fn with_a_ladder(said: &str) -> String {
        format!("{A_WHOLE_FILE}\n[connection]\n{said}\n")
    }

    #[test]
    fn refuses_a_file_that_is_not_there() {
        figment::Jail::expect_with(|jail| {
            let _ = jail;

            let Err(refusal) = Deployment::load(Path::new("voxloop.toml")) else {
                panic!("expected a refusal to start");
            };

            assert!(
                matches!(refusal, DeploymentError::Missing { .. }),
                "expected a refusal naming the missing file, got {refusal:?}"
            );
            Ok(())
        });
    }

    /// The one value with no fallback. A box that has not said where it is would come up,
    /// serve the console, fill its seats and carry no audio, and every ICE candidate it
    /// offered would name an address no client can reach.
    #[test]
    fn refuses_a_file_that_does_not_say_where_the_audio_is() {
        figment::Jail::expect_with(|jail| {
            jail.create_file(
                "voxloop.toml",
                r#"
                [tls]
                certificate = "/etc/voxloop/tls/fullchain.pem"
                private_key = "/etc/voxloop/tls/privkey.pem"

                [store]
                path = "/var/lib/voxloop/voxloop.sqlite"
                "#,
            )?;

            let Err(refusal) = Deployment::load(Path::new("voxloop.toml")) else {
                panic!("expected a refusal to start");
            };

            assert!(
                matches!(refusal, DeploymentError::Unreadable { .. }),
                "expected a refusal, got {refusal:?}"
            );
            Ok(())
        });
    }

    #[test]
    fn refuses_a_file_with_no_certificate_in_it() {
        figment::Jail::expect_with(|jail| {
            jail.create_file(
                "voxloop.toml",
                r#"
                [store]
                path = "/var/lib/voxloop/voxloop.sqlite"
                "#,
            )?;

            let Err(refusal) = Deployment::load(Path::new("voxloop.toml")) else {
                panic!("expected a refusal to start");
            };

            assert!(
                matches!(refusal, DeploymentError::Unreadable { .. }),
                "expected a refusal, got {refusal:?}"
            );
            Ok(())
        });
    }
}
