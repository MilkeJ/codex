use pretty_assertions::assert_eq;
use sqlx::AssertSqlSafe;
use sqlx::Connection;
use sqlx::Row;
use sqlx::SqlSafeStr;
use sqlx::migrate::MigrateError;
use sqlx::migrate::Migration;
use sqlx::migrate::MigrationType;
use sqlx::migrate::Migrator;
use sqlx::sqlite::SqlitePoolOptions;
use std::borrow::Cow;

use super::STATE_MIGRATOR;
use super::THREAD_HISTORY_MIGRATOR;
use super::clone_migrator_with_migrations;
use super::migrator_with_eol_compatible_checksums;
use super::repair_legacy_recency_migration_version;
use super::run_after_eol_compatibility_snapshot;
use super::run_migrator_with_eol_compatible_checksums;
use super::runtime_goals_migrator;
use super::runtime_logs_migrator;
use super::runtime_memories_migrator;
use super::runtime_state_migrator;
use crate::StateRuntime;
use crate::goals_db_path;
use crate::logs_db_path;
use crate::memories_db_path;
use crate::state_db_path;
use crate::thread_history_db_path;

fn migration_with_crlf(migration: &Migration) -> Migration {
    let lf_sql = migration
        .sql
        .as_str()
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    Migration::new(
        migration.version,
        migration.description.clone(),
        migration.migration_type,
        AssertSqlSafe(lf_sql.replace('\n', "\r\n")).into_sql_str(),
        migration.no_tx,
    )
}

fn migrator_with_crlf(migrator: &Migrator) -> Migrator {
    clone_migrator_with_migrations(
        migrator,
        migrator
            .migrations
            .iter()
            .map(migration_with_crlf)
            .collect(),
    )
}

fn test_migrator(migrations: &[(i64, &str)]) -> Migrator {
    Migrator::with_migrations(
        migrations
            .iter()
            .map(|(version, sql)| {
                Migration::new(
                    *version,
                    Cow::Owned(format!("test migration {version}")),
                    MigrationType::Simple,
                    AssertSqlSafe((*sql).to_string()).into_sql_str(),
                    /*no_tx*/ false,
                )
            })
            .collect(),
    )
}

async fn open_memory_pool() -> sqlx::SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory SQLite database should open")
}

async fn migration_history(pool: &sqlx::SqlitePool) -> Vec<(i64, Vec<u8>)> {
    sqlx::query_as(
        "SELECT version, checksum FROM _sqlx_migrations WHERE success = TRUE ORDER BY version",
    )
    .fetch_all(pool)
    .await
    .expect("migration history should load")
}

fn migrator_through(version: i64) -> Migrator {
    Migrator {
        migrations: Cow::Owned(
            STATE_MIGRATOR
                .migrations
                .iter()
                .filter(|migration| migration.version <= version)
                .cloned()
                .collect(),
        ),
        ignore_missing: STATE_MIGRATOR.ignore_missing,
        locking: STATE_MIGRATOR.locking,
        table_name: STATE_MIGRATOR.table_name.clone(),
        create_schemas: STATE_MIGRATOR.create_schemas.clone(),
        no_tx: STATE_MIGRATOR.no_tx,
    }
}

#[tokio::test]
async fn recency_migration_backfills_and_seeds_old_binary_inserts() {
    let sqlite_home = crate::runtime::test_support::unique_temp_dir();
    tokio::fs::create_dir_all(&sqlite_home)
        .await
        .expect("sqlite home should be created");
    let _cleanup = scopeguard::guard(sqlite_home.clone(), |sqlite_home| {
        let _ = std::fs::remove_dir_all(sqlite_home);
    });
    let sqlite = crate::SqliteConfig::new_for_testing(sqlite_home.clone());
    let pool = sqlite
        .open_read_write_pool(&state_db_path(&sqlite_home))
        .await
        .expect("sqlite database should open");
    migrator_through(/*version*/ 37)
        .run(&pool)
        .await
        .expect("pre-recency migrations should apply");

    sqlx::query(
        r#"
INSERT INTO threads (
    id,
    rollout_path,
    created_at,
    updated_at,
    created_at_ms,
    updated_at_ms,
    source,
    model_provider,
    cwd,
    title,
    sandbox_policy,
    approval_mode
) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind("00000000-0000-0000-0000-000000000001")
    .bind("/tmp/first.jsonl")
    .bind(1_700_000_000_i64)
    .bind(1_700_000_100_i64)
    .bind(1_700_000_000_123_i64)
    .bind(1_700_000_100_456_i64)
    .bind("cli")
    .bind("openai")
    .bind("/tmp")
    .bind("")
    .bind("read-only")
    .bind("on-request")
    .execute(&pool)
    .await
    .expect("legacy row should insert");

    STATE_MIGRATOR
        .run(&pool)
        .await
        .expect("recency migration should apply");

    let backfilled = sqlx::query(
        "SELECT updated_at, updated_at_ms, recency_at, recency_at_ms FROM threads WHERE id = ?",
    )
    .bind("00000000-0000-0000-0000-000000000001")
    .fetch_one(&pool)
    .await
    .expect("backfilled row should load");
    assert_eq!(backfilled.get::<i64, _>("recency_at"), 1_700_000_100);
    assert_eq!(backfilled.get::<i64, _>("recency_at_ms"), 1_700_000_100_456);

    sqlx::query(
        r#"
INSERT INTO threads (
    id,
    rollout_path,
    created_at,
    updated_at,
    created_at_ms,
    updated_at_ms,
    source,
    model_provider,
    cwd,
    title,
    sandbox_policy,
    approval_mode
) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind("00000000-0000-0000-0000-000000000002")
    .bind("/tmp/second.jsonl")
    .bind(1_700_000_200_i64)
    .bind(1_700_000_300_i64)
    .bind(1_700_000_200_123_i64)
    .bind(1_700_000_300_456_i64)
    .bind("cli")
    .bind("openai")
    .bind("/tmp")
    .bind("")
    .bind("read-only")
    .bind("on-request")
    .execute(&pool)
    .await
    .expect("old-binary row should insert");

    let seeded = sqlx::query("SELECT recency_at, recency_at_ms FROM threads WHERE id = ?")
        .bind("00000000-0000-0000-0000-000000000002")
        .fetch_one(&pool)
        .await
        .expect("old-binary row should load");
    assert_eq!(seeded.get::<i64, _>("recency_at"), 1_700_000_300);
    assert_eq!(seeded.get::<i64, _>("recency_at_ms"), 1_700_000_300_456);

    pool.close().await;
}

#[tokio::test]
async fn repairs_recency_migration_that_was_applied_as_version_38() {
    let sqlite_home = crate::runtime::test_support::unique_temp_dir();
    tokio::fs::create_dir_all(&sqlite_home)
        .await
        .expect("sqlite home should be created");
    let _cleanup = scopeguard::guard(sqlite_home.clone(), |sqlite_home| {
        let _ = std::fs::remove_dir_all(sqlite_home);
    });
    let sqlite = crate::SqliteConfig::new_for_testing(sqlite_home.clone());
    let pool = sqlite
        .open_read_write_pool(&state_db_path(&sqlite_home))
        .await
        .expect("sqlite database should open");
    migrator_through(/*version*/ 37)
        .run(&pool)
        .await
        .expect("pre-recency migrations should apply");

    let recency_migration = STATE_MIGRATOR
        .migrations
        .iter()
        .find(|migration| migration.version == 39)
        .expect("recency migration should exist");
    let mut legacy_migrations = STATE_MIGRATOR
        .migrations
        .iter()
        .filter(|migration| migration.version <= 37)
        .cloned()
        .collect::<Vec<_>>();
    legacy_migrations.push(Migration::new(
        38,
        recency_migration.description.clone(),
        recency_migration.migration_type,
        recency_migration.sql.clone(),
        recency_migration.no_tx,
    ));
    let legacy_recency_migrator = Migrator::with_migrations(legacy_migrations);
    legacy_recency_migrator
        .run(&pool)
        .await
        .expect("legacy recency migration should apply as version 38");

    repair_legacy_recency_migration_version(&pool, &STATE_MIGRATOR)
        .await
        .expect("legacy migration history should be repaired");
    STATE_MIGRATOR
        .run(&pool)
        .await
        .expect("current migrations should apply after repair");

    let applied = sqlx::query(
        "SELECT version, checksum FROM _sqlx_migrations WHERE version >= 38 ORDER BY version",
    )
    .fetch_all(&pool)
    .await
    .expect("applied migrations should load")
    .into_iter()
    .map(|row| {
        (
            row.get::<i64, _>("version"),
            row.get::<Vec<u8>, _>("checksum"),
        )
    })
    .collect::<Vec<_>>();
    let expected = STATE_MIGRATOR
        .migrations
        .iter()
        .filter(|migration| migration.version >= 38)
        .map(|migration| (migration.version, migration.checksum.to_vec()))
        .collect::<Vec<_>>();
    assert_eq!(applied, expected);

    pool.close().await;
}

#[tokio::test]
async fn repair_recency_migration_succeeds_while_another_connection_holds_writer_slot() {
    let sqlite_home = crate::runtime::test_support::unique_temp_dir();
    tokio::fs::create_dir_all(&sqlite_home)
        .await
        .expect("sqlite home should be created");
    let _cleanup = scopeguard::guard(sqlite_home.clone(), |sqlite_home| {
        let _ = std::fs::remove_dir_all(sqlite_home);
    });
    let sqlite = crate::SqliteConfig::new_for_testing(sqlite_home.clone());
    let state_path = state_db_path(&sqlite_home);
    let pool = sqlite
        .open_read_write_pool(&state_path)
        .await
        .expect("database should open");
    STATE_MIGRATOR
        .run(&pool)
        .await
        .expect("current migrations should apply");
    let read_pool = sqlite
        .open_read_only_pool(&state_path)
        .await
        .expect("read-only pool should open");
    let mut write_connection = pool.acquire().await.expect("write connection should open");
    let write_transaction = write_connection
        .begin_with("BEGIN IMMEDIATE")
        .await
        .expect("write transaction should acquire the writer slot");

    let repair_result = repair_legacy_recency_migration_version(&read_pool, &STATE_MIGRATOR).await;

    write_transaction
        .rollback()
        .await
        .expect("write transaction should roll back");
    drop(write_connection);
    read_pool.close().await;
    pool.close().await;
    repair_result.expect("current migration history should not need the writer slot");
}

#[tokio::test]
async fn crlf_applied_migration_is_accepted_by_lf_migrator_without_rewriting_history() {
    let pool = open_memory_pool().await;
    let crlf_migrator = test_migrator(&[(
        1,
        "CREATE TABLE first (value INTEGER);\r\nINSERT INTO first VALUES (1);\r\n",
    )]);
    crlf_migrator
        .run(&pool)
        .await
        .expect("CRLF migration should apply");
    let history_before = migration_history(&pool).await;

    let lf_migrator = test_migrator(&[
        (
            1,
            "CREATE TABLE first (value INTEGER);\nINSERT INTO first VALUES (1);\n",
        ),
        (2, "CREATE TABLE second (value INTEGER);\n"),
    ]);
    let compatible_migrator = migrator_with_eol_compatible_checksums(&pool, &lf_migrator)
        .await
        .expect("compatible migrator should be constructed");
    compatible_migrator
        .run(&pool)
        .await
        .expect("LF migrator should accept CRLF history");

    let history_after = migration_history(&pool).await;
    assert_eq!(
        history_after,
        vec![
            history_before[0].clone(),
            (2, lf_migrator.migrations[1].checksum.to_vec()),
        ]
    );
    pool.close().await;
}

#[tokio::test]
async fn lf_applied_migration_is_accepted_by_crlf_migrator_without_rewriting_history() {
    let pool = open_memory_pool().await;
    let lf_migrator = test_migrator(&[(
        1,
        "CREATE TABLE sample (value INTEGER);\nINSERT INTO sample VALUES (1);\n",
    )]);
    lf_migrator
        .run(&pool)
        .await
        .expect("LF migration should apply");
    let history_before = migration_history(&pool).await;

    let crlf_migrator = test_migrator(&[(
        1,
        "CREATE TABLE sample (value INTEGER);\r\nINSERT INTO sample VALUES (1);\r\n",
    )]);
    let compatible_migrator = migrator_with_eol_compatible_checksums(&pool, &crlf_migrator)
        .await
        .expect("compatible migrator should be constructed");
    compatible_migrator
        .run(&pool)
        .await
        .expect("CRLF migrator should accept LF history");

    assert_eq!(migration_history(&pool).await, history_before);
    pool.close().await;
}

#[tokio::test]
async fn eol_compatibility_rejects_genuine_sql_changes() {
    let pool = open_memory_pool().await;
    let original_migrator = test_migrator(&[(1, "CREATE TABLE sample (value INTEGER);\n")]);
    original_migrator
        .run(&pool)
        .await
        .expect("original migration should apply");
    let history_before = migration_history(&pool).await;

    let changed_migrator = test_migrator(&[(1, "CREATE TABLE sample (value TEXT);\n")]);
    let err = run_migrator_with_eol_compatible_checksums(&pool, &changed_migrator)
        .await
        .expect_err("genuine SQL changes must remain fatal");

    assert!(matches!(
        err.downcast_ref::<MigrateError>(),
        Some(MigrateError::VersionMismatch(1))
    ));
    assert_eq!(migration_history(&pool).await, history_before);
    pool.close().await;
}

#[tokio::test]
async fn eol_compatibility_retries_a_stale_applied_migration_snapshot_once() {
    let pool = open_memory_pool().await;
    let lf_migrator = test_migrator(&[(
        1,
        "CREATE TABLE sample (value INTEGER);\nINSERT INTO sample VALUES (1);\n",
    )]);
    let stale_compatible_migrator = migrator_with_eol_compatible_checksums(&pool, &lf_migrator)
        .await
        .expect("compatibility snapshot should be constructed before migration");
    let crlf_migrator = migrator_with_crlf(&lf_migrator);
    crlf_migrator
        .run(&pool)
        .await
        .expect("parallel CRLF migrator should apply after the stale snapshot");
    let history_before = migration_history(&pool).await;

    run_after_eol_compatibility_snapshot(&pool, &lf_migrator, stale_compatible_migrator)
        .await
        .expect("LF migrator should rebuild a stale compatibility snapshot");

    assert_eq!(migration_history(&pool).await, history_before);
    pool.close().await;
}

#[tokio::test]
async fn eol_compatibility_rejects_newlines_inside_quoted_tokens() {
    let cases = [
        (
            "string literal",
            "CREATE TABLE sample (value TEXT);\r\nINSERT INTO sample VALUES ('first\r\nsecond');\r\n",
            "CREATE TABLE sample (value TEXT);\nINSERT INTO sample VALUES ('first\nsecond');\n",
        ),
        (
            "quoted identifier",
            "CREATE TABLE \"first\r\nsecond\" (value INTEGER);\r\n",
            "CREATE TABLE \"first\nsecond\" (value INTEGER);\n",
        ),
        (
            "backtick-quoted identifier",
            "CREATE TABLE `first\r\nsecond` (value INTEGER);\r\n",
            "CREATE TABLE `first\nsecond` (value INTEGER);\n",
        ),
        (
            "bracket-quoted identifier",
            "CREATE TABLE [first\r\nsecond] (value INTEGER);\r\n",
            "CREATE TABLE [first\nsecond] (value INTEGER);\n",
        ),
    ];

    for (name, crlf_sql, lf_sql) in cases {
        let pool = open_memory_pool().await;
        test_migrator(&[(1, crlf_sql)])
            .run(&pool)
            .await
            .unwrap_or_else(|err| panic!("{name} CRLF migration should apply: {err}"));
        let history_before = migration_history(&pool).await;

        let lf_migrator = test_migrator(&[(1, lf_sql)]);
        let compatible_migrator = migrator_with_eol_compatible_checksums(&pool, &lf_migrator)
            .await
            .unwrap_or_else(|err| {
                panic!("{name} compatible migrator should be constructed: {err}")
            });
        let err = compatible_migrator
            .run(&pool)
            .await
            .expect_err("newline changes inside quoted tokens must remain fatal");

        assert!(matches!(err, MigrateError::VersionMismatch(1)), "{name}");
        assert_eq!(migration_history(&pool).await, history_before, "{name}");
        pool.close().await;
    }
}

#[tokio::test]
async fn state_runtime_opens_all_eager_databases_with_crlf_history_without_rewriting_it() {
    let sqlite_home = crate::runtime::test_support::unique_temp_dir();
    tokio::fs::create_dir_all(&sqlite_home)
        .await
        .expect("sqlite home should be created");
    let _cleanup = scopeguard::guard(sqlite_home.clone(), |sqlite_home| {
        let _ = std::fs::remove_dir_all(sqlite_home);
    });
    let sqlite = crate::SqliteConfig::new_for_testing(sqlite_home.clone());
    let databases = [
        (
            "state",
            state_db_path(&sqlite_home),
            runtime_state_migrator(),
        ),
        ("logs", logs_db_path(&sqlite_home), runtime_logs_migrator()),
        (
            "goals",
            goals_db_path(&sqlite_home),
            runtime_goals_migrator(),
        ),
        (
            "memories",
            memories_db_path(&sqlite_home),
            runtime_memories_migrator(),
        ),
    ];
    let mut expected_histories = Vec::new();
    for (name, path, migrator) in databases {
        let pool = sqlite
            .open_read_write_pool(&path)
            .await
            .unwrap_or_else(|err| panic!("{name} database should open: {err}"));
        migrator_with_crlf(&migrator)
            .run(&pool)
            .await
            .unwrap_or_else(|err| panic!("{name} CRLF migrations should apply: {err}"));
        expected_histories.push((name, path, migration_history(&pool).await));
        pool.close().await;
    }

    let runtime = StateRuntime::init(sqlite_home.clone(), "test-provider".to_string())
        .await
        .expect("state runtime should accept CRLF migration histories");
    runtime.close().await;

    for (name, path, expected_history) in expected_histories {
        let pool = sqlite
            .open_read_only_pool(&path)
            .await
            .unwrap_or_else(|err| panic!("{name} database should reopen: {err}"));
        assert_eq!(migration_history(&pool).await, expected_history, "{name}");
        pool.close().await;
    }
}

#[tokio::test]
async fn lazy_thread_history_open_accepts_crlf_history_without_rewriting_it() {
    let sqlite_home = crate::runtime::test_support::unique_temp_dir();
    tokio::fs::create_dir_all(&sqlite_home)
        .await
        .expect("sqlite home should be created");
    let _cleanup = scopeguard::guard(sqlite_home.clone(), |sqlite_home| {
        let _ = std::fs::remove_dir_all(sqlite_home);
    });
    let sqlite = crate::SqliteConfig::new_for_testing(sqlite_home.clone());
    let history_path = thread_history_db_path(&sqlite_home);
    let pool = sqlite
        .open_read_write_pool(&history_path)
        .await
        .expect("thread history database should open");
    migrator_with_crlf(&THREAD_HISTORY_MIGRATOR)
        .run(&pool)
        .await
        .expect("CRLF thread history migrations should apply");
    let history_before = migration_history(&pool).await;
    pool.close().await;

    let pool = crate::open_thread_history_db(&sqlite_home)
        .await
        .expect("lazy thread history open should accept CRLF history");
    assert_eq!(migration_history(&pool).await, history_before);
    pool.close().await;
}

#[tokio::test]
async fn repairs_crlf_legacy_recency_migration_without_rewriting_checksum() {
    let pool = open_memory_pool().await;
    migrator_through(/*version*/ 37)
        .run(&pool)
        .await
        .expect("pre-recency migrations should apply");

    let recency_migration = STATE_MIGRATOR
        .migrations
        .iter()
        .find(|migration| migration.version == 39)
        .expect("recency migration should exist");
    let crlf_recency_migration = migration_with_crlf(recency_migration);
    let legacy_checksum = crlf_recency_migration.checksum.to_vec();
    let mut legacy_migrations = STATE_MIGRATOR
        .migrations
        .iter()
        .filter(|migration| migration.version <= 37)
        .cloned()
        .collect::<Vec<_>>();
    legacy_migrations.push(Migration::new(
        38,
        crlf_recency_migration.description.clone(),
        crlf_recency_migration.migration_type,
        crlf_recency_migration.sql.clone(),
        crlf_recency_migration.no_tx,
    ));
    Migrator::with_migrations(legacy_migrations)
        .run(&pool)
        .await
        .expect("legacy CRLF recency migration should apply as version 38");

    repair_legacy_recency_migration_version(&pool, &STATE_MIGRATOR)
        .await
        .expect("legacy CRLF migration history should be repaired");
    let compatible_migrator =
        migrator_with_eol_compatible_checksums(&pool, &runtime_state_migrator())
            .await
            .expect("compatible state migrator should be constructed");
    compatible_migrator
        .run(&pool)
        .await
        .expect("current migrations should apply after CRLF repair");

    let repaired_checksum = sqlx::query_scalar::<_, Vec<u8>>(
        "SELECT checksum FROM _sqlx_migrations WHERE version = 39",
    )
    .fetch_one(&pool)
    .await
    .expect("repaired migration checksum should load");
    assert_eq!(repaired_checksum, legacy_checksum);
    pool.close().await;
}
