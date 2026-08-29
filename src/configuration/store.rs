use std::path::Path;
use std::time::Duration;

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

/// The one SQLite file the deployment persists into, open and migrated.
///
/// Opening is the whole of the story: the file is created if it is not there, put into
/// write-ahead-log mode, checked against the schema this binary knows, and migrated.
pub(crate) struct Store {
    pool: SqlitePool,
}

/// What can go wrong reaching the store, said without naming what is behind it.
#[derive(Debug, thiserror::Error)]
pub(crate) enum StoreError {
    #[error("the store could not be opened")]
    Unavailable(#[source] Box<dyn std::error::Error + Send + Sync>),

    /// A rollback that appeared to succeed and then wrote into a schema it misunderstood is
    /// the failure this refusal exists to prevent.
    #[error(
        "the store is at schema version {found} and this binary knows up to {known}, so it was \
         written by a newer VoxLoop"
    )]
    SchemaNewerThanBinary { found: i64, known: i64 },

    /// Not a failure of the store: a refusal the caller can act on and a human can read.
    #[error("the username {username:?} is already taken")]
    UsernameTaken { username: String },
}

/// Anything the store itself could not do, said without naming what is behind it.
pub(super) fn unavailable(error: impl std::error::Error + Send + Sync + 'static) -> StoreError {
    StoreError::Unavailable(Box::new(error))
}

/// Now, in milliseconds since the Unix epoch — the one shape of time this store holds.
///
/// An integer sorts and compares without a date library on either side of the wire, and
/// rendering one for a human to read is the console's job.
pub(super) fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
        })
}

/// The migrations this binary carries, run against the store at startup. Embedded, so a
/// customer upgrades by replacing one file.
static MIGRATIONS: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

impl Store {
    /// Open the store at `path`, creating it if it is not there, and migrate it.
    pub(crate) async fn open(path: &Path) -> Result<Self, StoreError> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5));

        let pool = SqlitePoolOptions::new()
            .connect_with(options)
            .await
            .map_err(unavailable)?;

        refuse_a_newer_schema(&pool).await?;

        MIGRATIONS.run(&pool).await.map_err(unavailable)?;

        tracing::info!(
            target: crate::telemetry::module::CONFIGURATION,
            path = %path.display(),
            "store open"
        );

        Ok(Self { pool })
    }

    /// Open a transaction for a caller to pass into repository methods.
    ///
    /// The handle is the caller's, deliberately: a grid edit reads the cell, writes it and
    /// writes the audit entry carrying its blast radius, and those have to land together
    /// or not at all ([ADR-0038]). A repository that opened its own transaction could not
    /// promise that.
    ///
    /// [ADR-0038]: ../../../docs/adr/0038-sqlite-behind-domain-shaped-repositories.md
    pub(crate) async fn begin(&self) -> Result<Transaction, StoreError> {
        self.pool
            .begin()
            .await
            .map(Transaction)
            .map_err(unavailable)
    }

    /// Close the store, letting SQLite finish its write-ahead log cleanly.
    pub(crate) async fn close(&self) {
        self.pool.close().await;
    }
}

/// One unit of persistent work, opened by a caller and passed into repository methods.
///
/// It is opaque on purpose. Nothing outside this module can tell what is behind it, which
/// is the whole of what makes the repository seam a seam ([ADR-0060]).
///
/// [ADR-0060]: ../../../docs/adr/0060-a-seam-names-domain-operations.md
pub(crate) struct Transaction(sqlx::Transaction<'static, sqlx::Sqlite>);

impl Transaction {
    /// The connection the repositories in this module run their statements on.
    ///
    /// `pub(super)` is the whole of the seam: every repository in `configuration` reaches it
    /// and nothing outside can, so no `sqlx` type ever crosses out ([ADR-0038]).
    ///
    /// [ADR-0038]: ../../../docs/adr/0038-sqlite-behind-domain-shaped-repositories.md
    pub(super) fn connection(&mut self) -> &mut sqlx::SqliteConnection {
        &mut self.0
    }

    /// Commit everything written through this handle.
    pub(crate) async fn commit(self) -> Result<(), StoreError> {
        self.0.commit().await.map_err(unavailable)
    }

    /// Abandon everything written through this handle.
    pub(crate) async fn roll_back(self) -> Result<(), StoreError> {
        self.0.rollback().await.map_err(unavailable)
    }
}

/// Refuse a store that some later binary has already migrated past us.
async fn refuse_a_newer_schema(pool: &SqlitePool) -> Result<(), StoreError> {
    let bookkeeping_exists: Option<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations'",
    )
    .fetch_optional(pool)
    .await
    .map_err(unavailable)?;

    if bookkeeping_exists.is_none() {
        return Ok(());
    }

    let stamped: Option<i64> = sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations")
        .fetch_one(pool)
        .await
        .map_err(unavailable)?;

    let known = MIGRATIONS
        .iter()
        .map(|migration| migration.version)
        .max()
        .unwrap_or(0);

    match stamped {
        Some(found) if found > known => Err(StoreError::SchemaNewerThanBinary { found, known }),
        _ => Ok(()),
    }
}

/// A store in a directory that exists for the length of one test.
///
/// Every test in the binary that needs persistence opens one of these: a temporary file,
/// migrated and thrown away ([ADR-0064]). There is no in-memory repository and there will
/// not be one.
///
/// [ADR-0064]: ../../../docs/adr/0064-tests-run-against-the-real-store.md
#[cfg(test)]
pub(crate) async fn a_temporary_store() -> (tempfile::TempDir, Store) {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let store = Store::open(&directory.path().join("voxloop.sqlite"))
        .await
        .expect("the store to open");

    (directory, store)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::{Connection, Row};

    /// Stamp a migration this binary has never heard of, as a newer VoxLoop would have.
    async fn stamp_a_future_migration(path: &Path, version: i64) {
        let mut writer = sqlx::SqliteConnection::connect(&format!("sqlite://{}", path.display()))
            .await
            .expect("a connection to the same file");
        sqlx::query(
            "INSERT INTO _sqlx_migrations \
             (version, description, installed_on, success, checksum, execution_time) \
             VALUES (?, 'from a newer binary', CURRENT_TIMESTAMP, 1, X'00', 0)",
        )
        .bind(version)
        .execute(&mut writer)
        .await
        .expect("the stamp to land");
    }

    #[tokio::test]
    async fn refuses_to_start_against_a_schema_newer_than_it_knows() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("voxloop.sqlite");
        drop(Store::open(&path).await.expect("the store to open"));
        stamp_a_future_migration(&path, 20_260_901_000_001).await;

        let Err(refusal) = Store::open(&path).await else {
            panic!("expected a refusal to start against a schema newer than this binary knows");
        };

        assert!(
            matches!(
                refusal,
                StoreError::SchemaNewerThanBinary {
                    found: 20_260_901_000_001,
                    ..
                }
            ),
            "expected a refusal naming the schema version, got {refusal:?}"
        );
    }

    #[tokio::test]
    async fn hands_out_a_transaction_the_caller_commits() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let store = Store::open(&dir.path().join("voxloop.sqlite"))
            .await
            .expect("the store to open");

        let transaction = store.begin().await.expect("a transaction");

        transaction
            .commit()
            .await
            .expect("the transaction to commit");
    }

    #[tokio::test]
    async fn opens_its_file_in_write_ahead_log_mode() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("voxloop.sqlite");

        let _store = Store::open(&path).await.expect("the store to open");

        let mut observer = sqlx::SqliteConnection::connect(&format!("sqlite://{}", path.display()))
            .await
            .expect("a second connection to the same file");
        let journal_mode: String = sqlx::query("PRAGMA journal_mode")
            .fetch_one(&mut observer)
            .await
            .expect("the journal mode")
            .get(0);

        assert_eq!(journal_mode, "wal");
    }
}
