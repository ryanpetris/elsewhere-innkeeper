use crate::{LaunchSettings, ScreenSize, Session};
use anyhow::{Context, Result, anyhow};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use rusqlite_migration::{M, Migrations};
use std::{
    collections::HashSet,
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
    sync::{Arc, Mutex},
};

fn initial_migration() -> M<'static> {
    M::up_with_hook(
        include_str!("../migrations/001_initial.sql"),
        |tx: &rusqlite::Transaction<'_>| {
            tx.execute(
                "INSERT INTO metadata VALUES (1, ?1)",
                [uuid::Uuid::new_v4().to_string()],
            )?;
            Ok(())
        },
    )
}

fn migrations() -> Migrations<'static> {
    Migrations::new(vec![initial_migration()])
}

#[derive(Clone, Debug)]
pub struct Store {
    connection: Arc<Mutex<Connection>>,
    pub installation_id: String,
}

impl Store {
    pub async fn open(path: PathBuf) -> Result<Self> {
        tokio::task::spawn_blocking(move || {
            // SQLite creates its journal files with the database's permissions.
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .open(&path)?;
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            // Closing another descriptor for this inode would release SQLite's process locks.
            drop(file);
            let mut connection = Connection::open(&path).context("Open session database")?;
            connection.busy_timeout(std::time::Duration::from_secs(5))?;
            connection.pragma_update(None, "journal_mode", "WAL")?;
            connection.pragma_update(None, "synchronous", "FULL")?;
            connection.pragma_update(None, "foreign_keys", "ON")?;
            migrations()
                .to_latest(&mut connection)
                .context("Migrate session database")?;
            let installation_id: String = connection
                .query_row(
                    "SELECT installation_id FROM metadata WHERE singleton = 1",
                    [],
                    |row| row.get(0),
                )
                .context("Read database installation_id")?;
            uuid::Uuid::parse_str(&installation_id).context("Invalid database installation_id")?;
            Ok(Self {
                connection: Arc::new(Mutex::new(connection)),
                installation_id,
            })
        })
        .await?
    }

    pub(crate) async fn run<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let connection = self.connection.clone();
        tokio::task::spawn_blocking(move || {
            let mut connection = connection
                .lock()
                .map_err(|_| anyhow!("Session database lock poisoned"))?;
            f(&mut connection)
        })
        .await?
    }

    pub async fn session(&self, id: &str) -> Result<Option<Session>> {
        let id = id.to_owned();
        self.run(move |db| {
            let tx = db.transaction()?;
            read_session(&tx, &id)
        })
        .await
    }

    pub async fn list(&self) -> Result<Vec<Session>> {
        self.run(|db| {
            let tx = db.transaction()?;
            let ids = tx
                .prepare_cached("SELECT id FROM sessions ORDER BY ordinal")?
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            ids.iter()
                .map(|id| read_session(&tx, id)?.context("Session disappeared during query"))
                .collect()
        })
        .await
    }

    #[cfg(test)]
    pub async fn create(&self, session: Session) -> Result<Option<Session>> {
        self.insert(session, None).await
    }
    pub async fn create_for(&self, session: Session, user: String) -> Result<Option<Session>> {
        self.insert(session, Some(user)).await
    }
    async fn insert(&self, mut session: Session, user: Option<String>) -> Result<Option<Session>> {
        self.run(move |db| {
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let used = tx
                .prepare_cached("SELECT port FROM sessions")?
                .query_map([], |row| row.get::<_, u16>(0))?
                .collect::<rusqlite::Result<HashSet<_>>>()?;
            let Some(port) = (19500..20000).find(|port| !used.contains(port)) else {
                return Ok(None);
            };
            session.port = port;
            write_session(&tx, &session)?;
            if let Some(user) = user {
                tx.execute(
                    "INSERT INTO session_access VALUES(?1,?2,'manager')",
                    params![session.id, user],
                )?;
            }
            tx.commit()?;
            Ok(Some(session))
        })
        .await
    }

    pub async fn change(
        &self,
        id: &str,
        f: impl FnOnce(&mut Session) + Send + 'static,
    ) -> Result<()> {
        let id = id.to_owned();
        self.run(move |db| {
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
            if let Some(mut session) = read_session(&tx, &id)? {
                let before = session.clone();
                f(&mut session);
                if session != before {
                    write_session(&tx, &session)?;
                }
            }
            tx.commit()?;
            Ok(())
        })
        .await
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        let id = id.to_owned();
        self.run(move |db| {
            db.execute("DELETE FROM sessions WHERE id = ?1", [id])?;
            Ok(())
        })
        .await
    }
}

fn read_session(db: &Connection, id: &str) -> Result<Option<Session>> {
    let session = db.prepare_cached(
        "SELECT id, name, distribution, port, started_ms, status, stage, error, installed_version,
         repair_available, version_error, upgrade_started_ms, upgrade_target, gpu_access, gpu, nvidia, configured, replacement FROM sessions WHERE id = ?1",
    )?.query_row(
        [id], |row| Ok(Session {
            id: row.get(0)?, name: row.get(1)?, distribution: row.get(2)?, port: row.get(3)?,
            started_ms: unsigned(row, 4)?, status: row.get(5)?, stage: row.get(6)?, error: row.get(7)?,
            installed_version: row.get(8)?, repair_available: row.get(9)?, version_error: row.get(10)?,
            upgrade_started_ms: unsigned(row, 11)?, upgrade_target: row.get(12)?, gpu_access: row.get(13)?,
            gpu: row.get::<_, Option<String>>(14)?.map(|json| serde_json::from_str(&json)).transpose().map_err(|e| rusqlite::Error::FromSqlConversionFailure(14, rusqlite::types::Type::Text, Box::new(e)))?,
            nvidia: row.get(15)?, configured: json_column(row, 16)?, replacement: json_column(row, 17)?,
            packages: vec![], docker_args: vec![], timings: Default::default(), startup_command: String::new(),
            screen_size: None, kiosk: false, software_encoding: false, applied_settings: None, launching_settings: None,
        }),
    ).optional()?;
    let Some(mut session) = session else {
        return Ok(None);
    };
    let mut desired = false;
    let mut statement = db.prepare_cached("SELECT kind, width, height, kiosk, startup_command, software_encoding FROM session_settings WHERE session_id = ?1")?;
    let settings = statement.query_map([id], |row| {
        let width: Option<u32> = row.get(1)?;
        let height: Option<u32> = row.get(2)?;
        Ok((
            row.get::<_, String>(0)?,
            LaunchSettings {
                screen_size: width
                    .zip(height)
                    .map(|(width, height)| ScreenSize { width, height }),
                kiosk: row.get(3)?,
                startup_command: row.get(4)?,
                software_encoding: row.get(5)?,
            },
        ))
    })?;
    for settings in settings {
        let (kind, settings) = settings?;
        match kind.as_str() {
            "desired" => {
                session.screen_size = settings.screen_size;
                session.kiosk = settings.kiosk;
                session.software_encoding = settings.software_encoding;
                session.startup_command = settings.startup_command;
                desired = true;
            }
            "applied" => session.applied_settings = Some(settings),
            "launching" => session.launching_settings = Some(settings),
            _ => return Err(anyhow!("Unknown settings kind")),
        }
    }
    anyhow::ensure!(desired, "Session has no desired settings");
    session.packages = db
        .prepare_cached(
            "SELECT name FROM session_packages WHERE session_id = ?1 ORDER BY position",
        )?
        .query_map([id], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    session.docker_args = db
        .prepare_cached(
            "SELECT argument FROM session_docker_args WHERE session_id = ?1 ORDER BY position",
        )?
        .query_map([id], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    session.timings = db
        .prepare_cached("SELECT stage, elapsed_ms FROM session_timings WHERE session_id = ?1")?
        .query_map([id], |row| Ok((row.get(0)?, unsigned(row, 1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(Some(session))
}

fn write_session(db: &Connection, s: &Session) -> Result<()> {
    db.execute(
        "INSERT INTO sessions (id, name, distribution, port, started_ms, status, stage, error,
         installed_version, repair_available, version_error, upgrade_started_ms, upgrade_target, gpu_access, gpu, nvidia, configured, replacement)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
         ON CONFLICT(id) DO UPDATE SET name=excluded.name, distribution=excluded.distribution,
         port=excluded.port, started_ms=excluded.started_ms, status=excluded.status, stage=excluded.stage,
         error=excluded.error, installed_version=excluded.installed_version, repair_available=excluded.repair_available,
         version_error=excluded.version_error, upgrade_started_ms=excluded.upgrade_started_ms, upgrade_target=excluded.upgrade_target, gpu_access=excluded.gpu_access, gpu=excluded.gpu, nvidia=excluded.nvidia, configured=excluded.configured, replacement=excluded.replacement",
        params![s.id, s.name, s.distribution, s.port, i64::try_from(s.started_ms)?, s.status, s.stage, s.error,
                s.installed_version, s.repair_available, s.version_error, i64::try_from(s.upgrade_started_ms)?, s.upgrade_target, s.gpu_access, s.gpu.as_ref().map(serde_json::to_string).transpose()?, s.nvidia,
                s.configured.as_ref().map(serde_json::to_string).transpose()?, s.replacement.as_ref().map(serde_json::to_string).transpose()?],
    )?;
    for table in [
        "session_settings",
        "session_packages",
        "session_timings",
        "session_docker_args",
    ] {
        db.execute(
            &format!("DELETE FROM {table} WHERE session_id = ?1"),
            [&s.id],
        )?;
    }
    for (kind, settings) in [
        ("desired", Some(LaunchSettings::from(s))),
        ("applied", s.applied_settings.clone()),
        ("launching", s.launching_settings.clone()),
    ] {
        if let Some(settings) = settings {
            db.execute(
                "INSERT INTO session_settings VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    s.id,
                    kind,
                    settings.screen_size.map(|size| size.width),
                    settings.screen_size.map(|size| size.height),
                    settings.kiosk,
                    settings.startup_command,
                    settings.software_encoding,
                ],
            )?;
        }
    }
    for (position, package) in s.packages.iter().enumerate() {
        db.execute(
            "INSERT INTO session_packages VALUES (?1, ?2, ?3)",
            params![s.id, position as i64, package],
        )?;
    }
    for (position, argument) in s.docker_args.iter().enumerate() {
        db.execute(
            "INSERT INTO session_docker_args VALUES (?1, ?2, ?3)",
            params![s.id, position as i64, argument],
        )?;
    }
    for (stage, elapsed) in &s.timings {
        db.execute(
            "INSERT INTO session_timings VALUES (?1, ?2, ?3)",
            params![s.id, stage, i64::try_from(*elapsed)?],
        )?;
    }
    Ok(())
}

fn json_column<T: serde::de::DeserializeOwned>(
    row: &rusqlite::Row<'_>,
    index: usize,
) -> rusqlite::Result<Option<T>> {
    row.get::<_, Option<String>>(index)?
        .map(|json| serde_json::from_str(&json))
        .transpose()
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                index,
                rusqlite::types::Type::Text,
                Box::new(e),
            )
        })
}

fn unsigned(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(index)?;
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("innkeeper-sqlite-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn database(&self) -> PathBuf {
            self.0.join("state.sqlite3")
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[tokio::test]
    async fn writes_remain_visible_after_an_external_reader_closes() {
        let directory = Directory::new();
        let store = Store::open(directory.database()).await.unwrap();
        let session = store.create(session()).await.unwrap().unwrap();
        let external_name = || {
            let output = std::process::Command::new("python3")
                .args([
                    "-c",
                    "import sqlite3, sys; db = sqlite3.connect(sys.argv[1]); print(db.execute('SELECT name FROM sessions WHERE id = ?', [sys.argv[2]]).fetchone()[0]); db.close()",
                ])
                .arg(directory.database())
                .arg(&session.id)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).unwrap().trim().to_owned()
        };
        assert_eq!(external_name(), session.name);
        store
            .change(&session.id, |s| s.name = "Updated".into())
            .await
            .unwrap();
        assert_eq!(external_name(), "Updated");
    }

    fn session() -> Session {
        let settings = LaunchSettings {
            screen_size: Some(ScreenSize {
                width: 640,
                height: 480,
            }),
            kiosk: true,
            software_encoding: true,
            startup_command: "foot\n--test 'quoted'".into(),
        };
        Session {
            id: uuid::Uuid::new_v4().to_string(),
            name: "SQLite session".into(),
            distribution: "arch".into(),
            packages: vec!["foot".into(), "firefox".into(), "foot".into()],
            docker_args: vec![
                "--security-opt=seccomp=unconfined".into(),
                "--security-opt=apparmor=unconfined".into(),
                "--cap-add=SYS_ADMIN".into(),
                "--cap-add=SYS_ADMIN".into(),
            ],
            startup_command: settings.startup_command.clone(),
            screen_size: settings.screen_size,
            kiosk: settings.kiosk,
            software_encoding: settings.software_encoding,
            nvidia: true,
            configured: None,
            replacement: None,
            gpu_access: true,
            gpu: Some(crate::gpu::Gpu {
                id: "0000:01:00.0".into(),
                driver: "nvidia".into(),
                node: "/dev/dri/renderD129".into(),
                major: 226,
                minor: 129,
            }),
            applied_settings: Some(settings.clone()),
            launching_settings: Some(settings),
            port: 0,
            started_ms: 42,
            status: "preparing".into(),
            stage: "launch".into(),
            error: Some("original error".into()),
            installed_version: Some("0.5.1-1".into()),
            repair_available: true,
            version_error: Some("fixture".into()),
            upgrade_started_ms: 21,
            upgrade_target: Some("0.5.1".into()),
            timings: [("download".into(), 123)].into(),
        }
    }

    #[tokio::test]
    async fn sessions_survive_restart_and_failed_writes_roll_back() {
        let directory = Directory::new();
        let db = Store::open(directory.database()).await.unwrap();
        let installation_id = db.installation_id.clone();
        let first = db.create(session()).await.unwrap().unwrap();
        let mut without_options = session();
        without_options.docker_args.clear();
        without_options.nvidia = false;
        without_options.gpu_access = false;
        without_options.gpu = None;
        without_options.distribution = "ubuntu".into();
        let second = db.create(without_options).await.unwrap().unwrap();
        assert!(db.session(&first.id).await.unwrap().unwrap() == first);
        assert!(
            db.change(&first.id, |s| {
                s.name = "Should roll back".into();
                s.docker_args.clear();
                s.screen_size = Some(ScreenSize {
                    width: 3,
                    height: 480,
                });
            })
            .await
            .is_err()
        );
        assert!(db.session(&first.id).await.unwrap().unwrap() == first);
        db.change(&first.id, |s| {
            s.configured = Some(crate::ContainerSettings::from(&*s));
            s.replacement = Some(crate::Replacement {
                source: "fixture-container".into(),
                snapshot: "fixture-snapshot".into(),
                image: Some("sha256:fixture".into()),
                settings: crate::ContainerSettings::from(&*s),
            });
            s.applied_settings = s.launching_settings.take();
        })
        .await
        .unwrap();
        let first = db.session(&first.id).await.unwrap().unwrap();
        assert!(first.launching_settings.is_none());
        let mut invalid = session();
        invalid.distribution = "invalid".into();
        assert!(db.create(invalid).await.is_err());
        drop(db);
        let db = Store::open(directory.database()).await.unwrap();
        assert_eq!(db.installation_id, installation_id);
        assert!(db.list().await.unwrap() == vec![first.clone(), second]);
        db.delete(&first.id).await.unwrap();
        let id = first.id.clone();
        db.run(move |connection| {
            for table in [
                "session_settings",
                "session_packages",
                "session_timings",
                "session_docker_args",
            ] {
                let count: i64 = connection.query_row(
                    &format!("SELECT count(*) FROM {table} WHERE session_id = ?1"),
                    [&id],
                    |r| r.get(0),
                )?;
                assert_eq!(count, 0);
            }
            Ok(())
        })
        .await
        .unwrap();
        let replacement = db.create(session()).await.unwrap().unwrap();
        assert_eq!(replacement.port, first.port);
        assert!(db.session(&first.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn concurrent_creates_and_changes_preserve_other_fields() {
        let directory = Directory::new();
        let db = Arc::new(Store::open(directory.database()).await.unwrap());
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..20 {
            let db = db.clone();
            tasks.spawn(async move { db.create(session()).await.unwrap().unwrap() });
        }
        let mut ports = HashSet::new();
        while let Some(result) = tasks.join_next().await {
            assert!(ports.insert(result.unwrap().port));
        }
        let id = db.list().await.unwrap()[0].id.clone();
        let mut updates = tokio::task::JoinSet::new();
        for index in 0..20 {
            let db = db.clone();
            let id = id.clone();
            updates.spawn(async move {
                db.change(&id, move |s| {
                    if index == 0 {
                        s.startup_command = "new settings".into();
                    }
                    s.timings.insert(format!("update-{index}"), index);
                })
                .await
                .unwrap();
            });
        }
        while let Some(result) = updates.join_next().await {
            result.unwrap();
        }
        let session = db.session(&id).await.unwrap().unwrap();
        assert_eq!(session.startup_command, "new settings");
        assert_eq!(session.timings.len(), 21);
        assert_ne!(
            session.applied_settings.unwrap().startup_command,
            session.startup_command
        );
    }

    #[tokio::test]
    async fn migrations_are_atomic_and_invalid_databases_are_rejected() {
        migrations().validate().unwrap();
        let mut connection = Connection::open_in_memory().unwrap();
        let mut definitions = vec![initial_migration()];
        definitions.push(M::up("INSERT INTO missing_table VALUES (1);"));
        assert!(
            Migrations::new(definitions)
                .to_latest(&mut connection)
                .is_err()
        );
        assert!(!connection.table_exists(None, "metadata").unwrap());
        let directory = Directory::new();
        let db = Store::open(directory.database()).await.unwrap();
        db.run(|connection| {
            connection.execute("DELETE FROM metadata", [])?;
            Ok(())
        })
        .await
        .unwrap();
        drop(db);
        assert!(Store::open(directory.database()).await.is_err());
        let newer = Directory::new();
        let db = Store::open(newer.database()).await.unwrap();
        db.run(|connection| {
            connection.pragma_update(None, "user_version", 2)?;
            Ok(())
        })
        .await
        .unwrap();
        drop(db);
        assert!(Store::open(newer.database()).await.is_err());
        let damaged = Directory::new();
        std::fs::write(damaged.database(), b"not a SQLite database").unwrap();
        assert!(Store::open(damaged.database()).await.is_err());
    }

    #[tokio::test]
    async fn accepted_operation_finishes_after_its_request_is_cancelled() {
        let directory = Directory::new();
        let db = Arc::new(Store::open(directory.database()).await.unwrap());
        let connection = db.connection.clone();
        let (locked_tx, locked_rx) = tokio::sync::oneshot::channel();
        let (unlock_tx, unlock_rx) = std::sync::mpsc::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            let _guard = connection.lock().unwrap();
            locked_tx.send(()).unwrap();
            unlock_rx.recv().unwrap();
        });
        locked_rx.await.unwrap();
        let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
        let (finished_tx, finished_rx) = tokio::sync::oneshot::channel();
        let worker_db = db.clone();
        let request = tokio::spawn(crate::finish_operation(async move {
            accepted_tx.send(()).unwrap();
            let session = worker_db.create(session()).await?.unwrap();
            finished_tx.send(session.id).unwrap();
            Ok(())
        }));
        accepted_rx.await.unwrap();
        request.abort();
        assert!(matches!(request.await, Err(error) if error.is_cancelled()));
        unlock_tx.send(()).unwrap();
        blocker.await.unwrap();
        let id = tokio::time::timeout(std::time::Duration::from_secs(5), finished_rx)
            .await
            .unwrap()
            .unwrap();
        assert!(db.session(&id).await.unwrap().is_some());
    }
}
