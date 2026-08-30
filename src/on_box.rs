//! The on-box CLI: what recovers a deployment nobody can sign into.
//!
//! **It bypasses every authorisation check in VoxLoop, by design** ([ADR-0025]). There is no
//! principal behind it, nothing to sign in as, and no requirement evaluated: being able to
//! run this binary against the deployment's store *is* the authorisation, which is the same
//! root of trust as the bootstrap code and the same statement said permanently — **shell
//! access to the host is the highest privilege in the system** (v1 §16).
//!
//! It is permanent rather than a first-run tool. With no mail path, the last administrator
//! locking themselves out is otherwise an unrecoverable deployment, and a bootstrap code is
//! not re-minted while somebody still holds the flag.
//!
//! It hands out an **enrolment code** rather than setting a password, which is not a
//! half-measure: an enrolment code is the only way a password is ever set in VoxLoop
//! (`CONTEXT.md`), and a second path that wrote one directly would be a second credential
//! flow to keep correct. It also keeps the password off a terminal, out of a shell history
//! and out of whatever is scraping the box's logs.
//!
//! Everything it does is audited, attributed to the CLI rather than to a person: there is no
//! person to attribute it to, and an entry claiming otherwise would be a lie in the one
//! record that must not hold any.
//!
//! It sits beside Transport at the top of the call graph rather than being a seam of its
//! own: it receives an invocation and calls Configuration, so nothing calls into it
//! ([ADR-0062]). It is enumerated in `docs/spec/api-surface.md` under *outside the model by
//! design*.
//!
//! [ADR-0025]: ../../docs/adr/0025-credentials-are-administered-because-there-is-no-email.md
//! [ADR-0062]: ../../docs/adr/0062-the-call-graph-is-acyclic-and-effects-modules-are-sinks.md

use std::path::PathBuf;

use crate::configuration::{
    AuditEntry, AuditEvent, AuditLog, BlastRadius, Change, ConfigurationWrite, Enrolment,
    EnrolmentCode, NewUser, Store, StoreError, Transaction, User, UserId, Users,
};

/// How the audit log names an act nobody signed in to perform.
///
/// The actor id is absent because there is no user behind it, and this is the name that
/// keeps the entry legible: somebody reading the log later needs *the box did this* rather
/// than a blank.
const THE_CLI: &str = "the on-box CLI";

/// The subcommand words, which are also what a first argument is tested against.
const ADMINISTRATOR: &str = "administrator";
const RESET_PASSWORD: &str = "reset-password";

/// What this invocation of the binary was asked to do.
pub(crate) enum Invocation {
    /// Serve, which is what the binary is for and what an empty command line means.
    Serve { deployment: PathBuf },
    /// Make the named user a system administrator, creating them if this box has never heard
    /// of them, and issue them an enrolment code.
    MakeAnAdministrator {
        deployment: PathBuf,
        username: String,
    },
    /// Take the named user's password away and issue them an enrolment code.
    ResetAPassword {
        deployment: PathBuf,
        username: String,
    },
    /// Say what the commands are, and say what running them means.
    Explain,
}

/// What stops a command that was understood.
#[derive(Debug, thiserror::Error)]
pub(crate) enum OnBoxError {
    #[error("`{command}` needs a username: voxloop {command} <username>")]
    NoUsername { command: String },

    #[error("--config needs a path to a deployment file")]
    NoDeploymentFile,

    #[error("this deployment has no user called {username:?}")]
    NoSuchUser { username: String },

    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Read the command line, without inventing an argument parser to do it.
///
/// The first word is a subcommand where it is one of the subcommand words and a deployment
/// file otherwise, which keeps `voxloop voxloop.toml` working exactly as it did. That is a
/// deliberate ambiguity resolved one way: a deployment file called `administrator` with no
/// extension would be read as the subcommand, and naming one that is a mistake this refuses
/// to accommodate.
pub(crate) fn invoked(
    arguments: impl IntoIterator<Item = String>,
) -> Result<Invocation, OnBoxError> {
    let mut arguments = arguments.into_iter();

    let Some(first) = arguments.next() else {
        return Ok(Invocation::Serve {
            deployment: deployment_file(None),
        });
    };

    match first.as_str() {
        "help" | "--help" | "-h" => Ok(Invocation::Explain),
        ADMINISTRATOR => {
            let (username, deployment) = rest_of(arguments, ADMINISTRATOR)?;
            Ok(Invocation::MakeAnAdministrator {
                deployment,
                username,
            })
        }
        RESET_PASSWORD => {
            let (username, deployment) = rest_of(arguments, RESET_PASSWORD)?;
            Ok(Invocation::ResetAPassword {
                deployment,
                username,
            })
        }
        _ => Ok(Invocation::Serve {
            deployment: deployment_file(Some(PathBuf::from(first))),
        }),
    }
}

/// The username a subcommand acts on, and the deployment file it was pointed at.
fn rest_of(
    arguments: impl Iterator<Item = String>,
    command: &str,
) -> Result<(String, PathBuf), OnBoxError> {
    let mut username = None;
    let mut deployment = None;
    let mut arguments = arguments.peekable();

    while let Some(argument) = arguments.next() {
        if argument == "--config" {
            deployment = Some(PathBuf::from(
                arguments.next().ok_or(OnBoxError::NoDeploymentFile)?,
            ));
        } else if username.is_none() {
            username = Some(argument);
        }
    }

    username
        .map(|username| (username, deployment_file(deployment)))
        .ok_or_else(|| OnBoxError::NoUsername {
            command: command.to_owned(),
        })
}

/// The deployment file named on the command line, in the environment, or by default.
fn deployment_file(named: Option<PathBuf>) -> PathBuf {
    named
        .or_else(|| std::env::var("VOXLOOP_CONFIG").ok().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(crate::DEPLOYMENT_FILE))
}

/// What a command did, in the form the operator needs it.
pub(crate) struct Enrolled {
    username: String,
    code: EnrolmentCode,
    /// Milliseconds since the Unix epoch.
    expires_at: i64,
    /// What the command did to the record before it issued the code.
    did: &'static str,
}

impl Enrolled {
    /// Say it, to whoever is standing at the box.
    ///
    /// The code goes to stdout rather than through `tracing`, because the operator asked for
    /// it and the deployment's log is not where a credential belongs — the bootstrap code is
    /// in a log because there is nobody at a terminal to hand it to, and here there is.
    pub(crate) fn say(&self) {
        println!(
            "\nvoxloop: {}\n\n    \
             Username        {}\n    \
             Enrolment code  {}\n    \
             Good for        {}\n\n\
             Hand that over out of band. It is single-use: redeeming it sets the password,\n\
             and it is the only way one is ever set.\n\n    \
             curl -k -X POST https://<this box>/api/enrolment \\\n      \
             -H 'content-type: application/json' \\\n      \
             -d '{{\"code\":\"{}\",\"password\":\"a long enough password\"}}'\n\n\
             Nothing checked whether you were entitled to do that. Shell access to this box\n\
             is the highest privilege in VoxLoop.\n",
            self.did,
            self.username,
            self.code.as_str(),
            how_long(self.expires_at),
            self.code.as_str(),
        );
    }
}

/// Roughly how long is left, said without a date library.
///
/// Every time in the store is milliseconds since the epoch, and rendering one for a human is
/// ordinarily the console's job. There is no console here, and *good for six days* is what
/// the person holding the code needs anyway.
fn how_long(expires_at: i64) -> String {
    let left = expires_at.saturating_sub(now());

    if left <= 0 {
        return "no time at all — this box's clock has moved".to_owned();
    }

    // Rounded up, because the operator is being told when to stop relying on it and the
    // half-hour either way is not what they are deciding with.
    let an_hour = 60 * 60 * 1_000;
    let hours = (left + an_hour - 1) / an_hour;

    if hours < 24 {
        how_many(hours, "hour")
    } else {
        how_many(hours / 24, "day")
    }
}

fn how_many(count: i64, unit: &str) -> String {
    if count == 1 {
        format!("1 {unit}")
    } else {
        format!("{count} {unit}s")
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
        })
}

/// Make the named user a system administrator, and issue them an enrolment code.
///
/// It creates the record where the box has never heard of the name, gives the flag where it
/// is not held, and **unlocks the account**, because the case this exists for is the last
/// administrator having locked themselves out and there is nobody left to unlock them.
///
/// It is idempotent in the way that matters: run twice, the second run changes nothing about
/// the record and issues a fresh code, invalidating the first.
pub(crate) async fn make_an_administrator(
    store: &Store,
    username: &str,
) -> Result<Enrolled, OnBoxError> {
    let mut transaction = store.begin().await?;

    let (user, did) = match transaction.user_named(username).await? {
        Some(found) => {
            let id = found.id.clone();
            let opened = open_the_way_in(&mut transaction, &found).await?;
            (id, opened)
        }
        None => {
            let created = create_an_administrator(&mut transaction, username).await?;
            (created, "made a system administrator")
        }
    };

    let enrolled = enrol(&mut transaction, &user, username, did).await?;
    transaction.commit().await?;

    Ok(enrolled)
}

/// Take the named user's password away, and issue them an enrolment code.
///
/// Taking it away is a forced password reset and ends every sign-in they hold, exactly as the
/// console's is: a reset that leaves the old session talking is not a reset ([ADR-0025]).
///
/// [ADR-0025]: ../../docs/adr/0025-credentials-are-administered-because-there-is-no-email.md
pub(crate) async fn reset_a_password(
    store: &Store,
    username: &str,
) -> Result<Enrolled, OnBoxError> {
    let mut transaction = store.begin().await?;

    let Some(user) = transaction.user_named(username).await? else {
        transaction.roll_back().await?;
        return Err(OnBoxError::NoSuchUser {
            username: username.to_owned(),
        });
    };

    if let Some(change) = transaction.clear_password(&user.id).await? {
        record(&mut transaction, AuditEvent::PasswordResetForced, &change).await?;
    }

    let enrolled = enrol(
        &mut transaction,
        &user.id,
        &user.username,
        "password taken away, and every sign-in it held ended",
    )
    .await?;
    transaction.commit().await?;

    Ok(enrolled)
}

/// Give the flag and take the lock off, recording whichever of the two was needed.
async fn open_the_way_in(
    transaction: &mut Transaction,
    found: &User,
) -> Result<&'static str, OnBoxError> {
    let mut did = "already a system administrator";

    if !found.is_system_administrator
        && let Some(change) = transaction
            .set_system_administration(&found.id, true)
            .await
            .map_err(store_fault)?
    {
        record(transaction, AuditEvent::UserEdited, &change).await?;
        did = "made a system administrator";
    }

    if found.is_locked
        && let Some(change) = transaction
            .set_account_lock(&found.id, false)
            .await
            .map_err(store_fault)?
    {
        record(transaction, AuditEvent::AccountUnlocked, &change).await?;
        did = "unlocked and made a system administrator";
    }

    Ok(did)
}

async fn create_an_administrator(
    transaction: &mut Transaction,
    username: &str,
) -> Result<UserId, OnBoxError> {
    let id = transaction
        .create_user(NewUser {
            username: username.to_owned(),
            // No password. The enrolment code below is what sets one, which is the same path
            // every other user on the deployment takes.
            password_hash: None,
            is_system_administrator: true,
        })
        .await
        .map_err(store_fault)?;

    let Some(created) = transaction.user(&id).await? else {
        // Unreachable: the record was written through this very transaction.
        return Ok(id);
    };

    record(
        transaction,
        AuditEvent::UserCreated,
        &Change {
            before: created.clone(),
            after: Some(created),
        },
    )
    .await?;

    Ok(id)
}

/// Issue the code, record the issue, and gather what the operator is told.
async fn enrol(
    transaction: &mut Transaction,
    user: &UserId,
    username: &str,
    did: &'static str,
) -> Result<Enrolled, OnBoxError> {
    let issued = transaction.issue_enrolment_code(user).await?;

    transaction
        .record(AuditEntry {
            event: AuditEvent::EnrolmentCodeIssued,
            actor: None,
            actor_name: THE_CLI.to_owned(),
            source: None,
            write: Some(issued.to_the_code(user, username, nothing_live())),
            operation: None,
        })
        .await?;

    Ok(Enrolled {
        username: username.to_owned(),
        code: issued.code,
        expires_at: issued.outstanding.expires_at,
        did,
    })
}

/// What a write from here does to anything live, which is nothing it can know about.
///
/// This process is not the one serving the deployment — it is what somebody reaches for when
/// that one is not answering them — so there is no state authority here to compute a radius
/// from. An empty one is the honest answer rather than a placeholder ([ADR-0039]).
///
/// [ADR-0039]: ../../docs/adr/0039-live-state-is-in-process-behind-one-state-authority.md
fn nothing_live() -> BlastRadius {
    BlastRadius::nothing_live()
}

/// Record a write this binary made with nobody signed in to make it.
async fn record(
    transaction: &mut Transaction,
    event: AuditEvent,
    change: &Change,
) -> Result<(), StoreError> {
    transaction
        .record(AuditEntry {
            event,
            // There is no principal. The absence is the record: an entry with no actor id
            // and this name is the CLI, and nothing else in VoxLoop writes one.
            actor: None,
            actor_name: THE_CLI.to_owned(),
            source: None,
            write: Some(ConfigurationWrite::to_a_user(change, nothing_live())),
            operation: None,
        })
        .await
}

/// A refusal the CLI cannot meet is a fault, because the CLI meets every requirement.
///
/// `LastSystemAdministrator` guards acts that take an administrator away and nothing here
/// takes one away; a name already taken cannot happen against a name just read back as
/// absent. What is left is the store, and it is reported as itself.
fn store_fault(refusal: crate::configuration::AdministrationRefused) -> OnBoxError {
    match refusal {
        crate::configuration::AdministrationRefused::Store(error) => OnBoxError::Store(error),
        other => OnBoxError::Store(crate::configuration::StoreError::Unavailable(Box::new(
            other,
        ))),
    }
}

/// What the commands are, and what running one means.
pub(crate) fn explain() {
    // The command words are interpolated rather than typed out, so the two cannot drift, and
    // the column is padded to a width rather than by counting spaces here.
    let commands = [
        (
            "voxloop [<deployment file>]".to_owned(),
            "serve this deployment",
        ),
        (
            format!("voxloop {ADMINISTRATOR} <username>"),
            "make or promote a system administrator",
        ),
        (
            format!("voxloop {RESET_PASSWORD} <username>"),
            "take a password away and issue a code",
        ),
        ("voxloop help".to_owned(), "this"),
    ];
    let listed = commands
        .iter()
        .map(|(command, what)| format!("    {command:<36}{what}"))
        .collect::<Vec<_>>()
        .join("\n");

    println!(
        "\
voxloop — a software voice loop system.

{listed}

Both commands print a single-use enrolment code. Hand it over out of band; redeeming it
sets the password, which is the only way one is ever set in VoxLoop. The code is redeemed
over HTTPS, so this is a way back into a deployment that is still serving. Point either at
a deployment file with --config <file>, or through VOXLOOP_CONFIG; otherwise they read
{deployment} in the working directory, exactly as serving does.

`{ADMINISTRATOR}` also unlocks the account. It has to: the last-administrator rule counts
flag holders and nothing else, so a box with two administrators can have both of them
locked and nobody left to unlock either.

These two commands run outside VoxLoop's authorisation model entirely. They evaluate no
requirement, resolve no principal and answer to nobody: being able to run this binary
against the deployment's store is the whole of the authorisation. That is deliberate and
permanent — with no mail path, the last administrator locking themselves out would
otherwise be an unrecoverable deployment — and it means shell access to this box is the
highest privilege in the system. Everything they do is written to the audit log,
attributed to the CLI rather than to a person, because there is no person to attribute it
to.
",
        deployment = crate::DEPLOYMENT_FILE,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::a_temporary_store;

    fn arguments(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn an_empty_command_line_serves_the_deployment_file_in_the_working_directory() {
        let Ok(Invocation::Serve { deployment }) = invoked(arguments(&[])) else {
            panic!("expected an empty command line to serve");
        };

        assert_eq!(deployment, PathBuf::from(crate::DEPLOYMENT_FILE));
    }

    #[test]
    fn a_first_argument_that_is_not_a_command_is_still_the_deployment_file() {
        let Ok(Invocation::Serve { deployment }) = invoked(arguments(&["/etc/voxloop.toml"]))
        else {
            panic!("expected a path to serve");
        };

        assert_eq!(deployment, PathBuf::from("/etc/voxloop.toml"));
    }

    #[test]
    fn reads_the_commands_and_the_user_each_one_acts_on() {
        let Ok(Invocation::MakeAnAdministrator { username, .. }) =
            invoked(arguments(&[ADMINISTRATOR, "flight"]))
        else {
            panic!("expected an administrator to be made");
        };
        assert_eq!(username, "flight");

        let Ok(Invocation::ResetAPassword { username, .. }) =
            invoked(arguments(&[RESET_PASSWORD, "flight"]))
        else {
            panic!("expected a password to be reset");
        };
        assert_eq!(username, "flight");
    }

    #[test]
    fn a_command_can_be_pointed_at_a_deployment_file() {
        let Ok(Invocation::MakeAnAdministrator {
            deployment,
            username,
        }) = invoked(arguments(&[
            ADMINISTRATOR,
            "flight",
            "--config",
            "/etc/voxloop.toml",
        ]))
        else {
            panic!("expected an administrator to be made");
        };

        assert_eq!(username, "flight");
        assert_eq!(deployment, PathBuf::from("/etc/voxloop.toml"));
    }

    #[test]
    fn a_command_with_no_username_says_what_it_needs() {
        assert!(matches!(
            invoked(arguments(&[ADMINISTRATOR])),
            Err(OnBoxError::NoUsername { .. })
        ));
        assert!(matches!(
            invoked(arguments(&[
                RESET_PASSWORD,
                "--config",
                "/etc/voxloop.toml"
            ])),
            Err(OnBoxError::NoUsername { .. })
        ));
    }

    #[test]
    fn every_way_of_asking_for_help_is_answered() {
        for asked in ["help", "--help", "-h"] {
            assert!(
                matches!(invoked(arguments(&[asked])), Ok(Invocation::Explain)),
                "{asked} was not understood as asking for help"
            );
        }
    }

    #[tokio::test]
    async fn makes_an_administrator_on_a_box_that_has_never_heard_of_them() {
        let (_directory, store) = a_temporary_store().await;

        let enrolled = make_an_administrator(&store, "flight")
            .await
            .expect("an administrator");

        let mut transaction = store.begin().await.expect("a transaction");
        let made = transaction
            .user_named("flight")
            .await
            .expect("the read to answer")
            .expect("a user");
        assert!(made.is_system_administrator);
        assert!(!made.has_password, "the CLI set a password itself");
        assert_eq!(
            transaction
                .spend_enrolment_code(&enrolled.code)
                .await
                .expect("the code to be spendable"),
            Some(made.id)
        );
    }

    #[tokio::test]
    async fn promotes_and_unlocks_somebody_the_box_already_holds() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let id = transaction
            .create_user(NewUser {
                username: "flight".to_owned(),
                password_hash: None,
                is_system_administrator: false,
            })
            .await
            .expect("a user");
        transaction
            .set_account_lock(&id, true)
            .await
            .expect("the lock to land");
        transaction.commit().await.expect("the user to land");

        make_an_administrator(&store, "flight")
            .await
            .expect("an administrator");

        let mut transaction = store.begin().await.expect("a transaction");
        let made = transaction
            .user(&id)
            .await
            .expect("the read to answer")
            .expect("a user");
        assert!(made.is_system_administrator);
        assert!(!made.is_locked, "the account is still locked");
    }

    #[tokio::test]
    async fn a_second_run_issues_a_fresh_code_and_invalidates_the_first() {
        let (_directory, store) = a_temporary_store().await;
        let first = make_an_administrator(&store, "flight")
            .await
            .expect("a code");

        let second = make_an_administrator(&store, "flight")
            .await
            .expect("another code");

        let mut transaction = store.begin().await.expect("a transaction");
        assert_eq!(
            transaction
                .spend_enrolment_code(&first.code)
                .await
                .expect("the read to answer"),
            None,
            "the first code still works"
        );
        assert!(
            transaction
                .spend_enrolment_code(&second.code)
                .await
                .expect("the read to answer")
                .is_some()
        );
    }

    #[tokio::test]
    async fn resetting_a_password_takes_it_away_and_ends_every_sign_in() {
        use crate::configuration::{PasswordHash, SignIns};

        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let id = transaction
            .create_user(NewUser {
                username: "flight".to_owned(),
                password_hash: Some(PasswordHash::already_hashed(
                    "$argon2id$stand-in".to_owned(),
                )),
                is_system_administrator: false,
            })
            .await
            .expect("a user");
        let token = transaction
            .open_sign_in(&id)
            .await
            .expect("a sign-in to open");
        transaction.commit().await.expect("the user to land");

        let enrolled = reset_a_password(&store, "flight").await.expect("a code");

        let mut transaction = store.begin().await.expect("a transaction");
        let reset = transaction
            .user(&id)
            .await
            .expect("the read to answer")
            .expect("a user");
        assert!(!reset.has_password);
        assert_eq!(
            transaction
                .holder_of(&token)
                .await
                .expect("the read to answer"),
            None,
            "a sign-in survived the reset"
        );
        assert_eq!(
            transaction
                .spend_enrolment_code(&enrolled.code)
                .await
                .expect("the code to be spendable"),
            Some(id)
        );
    }

    #[tokio::test]
    async fn resetting_a_password_for_somebody_the_box_has_never_heard_of_says_so() {
        let (_directory, store) = a_temporary_store().await;

        assert!(matches!(
            reset_a_password(&store, "nobody").await,
            Err(OnBoxError::NoSuchUser { .. })
        ));
    }

    #[tokio::test]
    async fn everything_it_does_is_audited_and_attributed_to_the_box_rather_than_a_person() {
        let (_directory, store) = a_temporary_store().await;

        make_an_administrator(&store, "flight")
            .await
            .expect("an administrator");

        let mut transaction = store.begin().await.expect("a transaction");
        let entries = transaction
            .recent_entries(10)
            .await
            .expect("the log to be readable");

        let events: Vec<AuditEvent> = entries.iter().map(|entry| entry.event).collect();
        assert_eq!(
            events,
            vec![AuditEvent::EnrolmentCodeIssued, AuditEvent::UserCreated]
        );
        for entry in &entries {
            assert_eq!(entry.actor, None, "the CLI claimed to be somebody");
            assert_eq!(entry.actor_name, THE_CLI);
        }
    }

    #[test]
    fn says_how_long_a_code_is_good_for_without_a_date_library() {
        let hour = 60 * 60 * 1_000;

        assert_eq!(how_long(now() + 7 * 24 * hour), "7 days");
        assert_eq!(how_long(now() + 3 * hour), "3 hours");
        assert_eq!(how_long(now() + 60_000), "1 hour");
        assert!(how_long(now() - hour).contains("clock"));
    }
}
