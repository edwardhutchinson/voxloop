use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};

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

        Figment::new()
            .merge(Toml::file(path))
            .merge(Env::prefixed("VOXLOOP_").split("__"))
            .extract()
            .map_err(|error| DeploymentError::Unreadable {
                path: path.to_path_buf(),
                detail: error.to_string(),
            })
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
