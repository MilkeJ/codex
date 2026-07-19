use std::borrow::Cow;

use sqlx::AssertSqlSafe;
use sqlx::Row;
use sqlx::SqlSafeStr;
use sqlx::SqlitePool;
use sqlx::migrate::MigrateError;
use sqlx::migrate::Migration;
use sqlx::migrate::Migrator;

pub(crate) static STATE_MIGRATOR: Migrator = sqlx::migrate!("./migrations");
pub(crate) static LOGS_MIGRATOR: Migrator = sqlx::migrate!("./logs_migrations");
pub(crate) static GOALS_MIGRATOR: Migrator = sqlx::migrate!("./goals_migrations");
pub(crate) static MEMORIES_MIGRATOR: Migrator = sqlx::migrate!("./memory_migrations");
pub(crate) static THREAD_HISTORY_MIGRATOR: Migrator = sqlx::migrate!("./thread_history_migrations");

/// Allow an older Codex binary to open a database that has already been
/// migrated by a newer binary running in parallel.
///
/// We intentionally ignore applied migration versions that are newer than the
/// embedded migration set. Known migration versions are still validated by
/// checksum, so this only relaxes the "database is ahead of me" case.
fn runtime_migrator(base: &'static Migrator) -> Migrator {
    Migrator {
        migrations: Cow::Borrowed(base.migrations.as_ref()),
        ignore_missing: true,
        locking: base.locking,
        no_tx: base.no_tx,
        table_name: base.table_name.clone(),
        create_schemas: base.create_schemas.clone(),
    }
}

pub(crate) fn runtime_state_migrator() -> Migrator {
    runtime_migrator(&STATE_MIGRATOR)
}

pub(crate) fn runtime_logs_migrator() -> Migrator {
    runtime_migrator(&LOGS_MIGRATOR)
}

pub(crate) fn runtime_goals_migrator() -> Migrator {
    runtime_migrator(&GOALS_MIGRATOR)
}

pub(crate) fn runtime_memories_migrator() -> Migrator {
    runtime_migrator(&MEMORIES_MIGRATOR)
}

// The paginated history projector will call this when it takes ownership of opening the database.
#[allow(dead_code)]
pub(crate) fn runtime_thread_history_migrator() -> Migrator {
    runtime_migrator(&THREAD_HISTORY_MIGRATOR)
}

/// Return a runtime migrator that accepts checksums produced by an equivalent
/// LF or CRLF checkout of already-applied migration SQL.
///
/// SQLx hashes the migration source bytes, so Git line-ending conversion can
/// otherwise make unchanged SQL appear modified. This changes only the
/// in-memory checksum used for validation. Unknown and pending migrations are
/// untouched, and the database's migration history is never rewritten.
pub(crate) async fn migrator_with_eol_compatible_checksums(
    pool: &SqlitePool,
    migrator: &Migrator,
) -> anyhow::Result<Migrator> {
    let mut migrations = migrator.migrations.to_vec();
    if !migrations_table_exists(pool).await? {
        return Ok(clone_migrator_with_migrations(migrator, migrations));
    }

    let applied_migrations = sqlx::query(
        "SELECT version, checksum FROM _sqlx_migrations WHERE success = TRUE ORDER BY version",
    )
    .fetch_all(pool)
    .await?;
    for row in applied_migrations {
        let version = row.try_get::<i64, _>("version")?;
        let stored_checksum = row.try_get::<Vec<u8>, _>("checksum")?;
        let Some(migration) = migrations
            .iter_mut()
            .find(|migration| migration.version == version)
        else {
            continue;
        };
        if migration.checksum.as_ref() != stored_checksum.as_slice()
            && checksum_matches_eol_variant(migration, stored_checksum.as_slice())
        {
            migration.checksum = Cow::Owned(stored_checksum);
        }
    }

    Ok(clone_migrator_with_migrations(migrator, migrations))
}

/// Run migrations with EOL-compatible validation, rebuilding the compatibility
/// snapshot once if another process changes the applied migration set between
/// the snapshot and SQLx's validation.
pub(crate) async fn run_migrator_with_eol_compatible_checksums(
    pool: &SqlitePool,
    migrator: &Migrator,
) -> anyhow::Result<()> {
    let compatible_migrator = migrator_with_eol_compatible_checksums(pool, migrator).await?;
    run_after_eol_compatibility_snapshot(pool, migrator, compatible_migrator).await
}

async fn run_after_eol_compatibility_snapshot(
    pool: &SqlitePool,
    migrator: &Migrator,
    compatible_migrator: Migrator,
) -> anyhow::Result<()> {
    match compatible_migrator.run(pool).await {
        Ok(()) => Ok(()),
        Err(MigrateError::VersionMismatch(_)) => {
            let compatible_migrator =
                migrator_with_eol_compatible_checksums(pool, migrator).await?;
            compatible_migrator
                .run(pool)
                .await
                .map_err(anyhow::Error::from)
        }
        Err(err) => Err(err.into()),
    }
}

fn clone_migrator_with_migrations(migrator: &Migrator, migrations: Vec<Migration>) -> Migrator {
    Migrator {
        migrations: Cow::Owned(migrations),
        ignore_missing: migrator.ignore_missing,
        locking: migrator.locking,
        no_tx: migrator.no_tx,
        table_name: migrator.table_name.clone(),
        create_schemas: migrator.create_schemas.clone(),
    }
}

fn checksum_matches_eol_variant(migration: &Migration, checksum: &[u8]) -> bool {
    if !eol_normalization_preserves_quoted_tokens(migration.sql.as_str()) {
        return false;
    }

    let lf_sql = normalize_sql_to_lf(migration.sql.as_str());
    if migration_checksum(migration, lf_sql.clone().into_owned()) == checksum {
        return true;
    }

    let crlf_sql = lf_sql.replace('\n', "\r\n");
    migration_checksum(migration, crlf_sql) == checksum
}

fn eol_normalization_preserves_quoted_tokens(sql: &str) -> bool {
    #[derive(Clone, Copy)]
    enum LexicalState {
        Normal,
        SingleQuoted,
        DoubleQuoted,
        BacktickQuoted,
        BracketQuoted,
        LineComment,
        BlockComment,
    }

    let bytes = sql.as_bytes();
    let mut state = LexicalState::Normal;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        let next = bytes.get(index + 1).copied();
        match state {
            LexicalState::Normal => match (byte, next) {
                (b'\'', _) => state = LexicalState::SingleQuoted,
                (b'"', _) => state = LexicalState::DoubleQuoted,
                (b'`', _) => state = LexicalState::BacktickQuoted,
                (b'[', _) => state = LexicalState::BracketQuoted,
                (b'-', Some(b'-')) => {
                    state = LexicalState::LineComment;
                    index += 1;
                }
                (b'/', Some(b'*')) => {
                    state = LexicalState::BlockComment;
                    index += 1;
                }
                _ => {}
            },
            LexicalState::SingleQuoted => {
                if matches!(byte, b'\r' | b'\n') {
                    return false;
                }
                if byte == b'\'' {
                    if next == Some(b'\'') {
                        index += 1;
                    } else {
                        state = LexicalState::Normal;
                    }
                }
            }
            LexicalState::DoubleQuoted => {
                if matches!(byte, b'\r' | b'\n') {
                    return false;
                }
                if byte == b'"' {
                    if next == Some(b'"') {
                        index += 1;
                    } else {
                        state = LexicalState::Normal;
                    }
                }
            }
            LexicalState::BacktickQuoted => {
                if matches!(byte, b'\r' | b'\n') {
                    return false;
                }
                if byte == b'`' {
                    if next == Some(b'`') {
                        index += 1;
                    } else {
                        state = LexicalState::Normal;
                    }
                }
            }
            LexicalState::BracketQuoted => {
                if matches!(byte, b'\r' | b'\n') {
                    return false;
                }
                if byte == b']' {
                    state = LexicalState::Normal;
                }
            }
            LexicalState::LineComment => {
                if matches!(byte, b'\r' | b'\n') {
                    state = LexicalState::Normal;
                }
            }
            LexicalState::BlockComment => {
                if byte == b'*' && next == Some(b'/') {
                    state = LexicalState::Normal;
                    index += 1;
                }
            }
        }
        index += 1;
    }
    true
}

fn normalize_sql_to_lf(sql: &str) -> Cow<'_, str> {
    if sql.contains('\r') {
        Cow::Owned(sql.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        Cow::Borrowed(sql)
    }
}

fn migration_checksum(migration: &Migration, sql: String) -> Vec<u8> {
    // SQLx does not publicly re-export its checksum helper. Constructing a
    // migration delegates to the same SHA-384 implementation used by SQLx.
    Migration::new(
        migration.version,
        migration.description.clone(),
        migration.migration_type,
        AssertSqlSafe(sql).into_sql_str(),
        migration.no_tx,
    )
    .checksum
    .into_owned()
}

async fn migrations_table_exists(pool: &SqlitePool) -> anyhow::Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations'",
    )
    .fetch_optional(pool)
    .await?
    .is_some())
}

pub(crate) async fn repair_legacy_recency_migration_version(
    pool: &SqlitePool,
    migrator: &Migrator,
) -> anyhow::Result<()> {
    let Some(recency_migration) = migrator
        .migrations
        .iter()
        .find(|migration| migration.version == 39)
    else {
        return Ok(());
    };
    if !migrations_table_exists(pool).await? {
        return Ok(());
    }

    let legacy_recency_checksum = sqlx::query_scalar::<_, Vec<u8>>(
        r#"
SELECT checksum
FROM _sqlx_migrations
WHERE version = ?
  AND NOT EXISTS (
      SELECT 1 FROM _sqlx_migrations WHERE version = ?
  )
        "#,
    )
    .bind(38_i64)
    .bind(recency_migration.version)
    .fetch_optional(pool)
    .await?;
    let Some(legacy_recency_checksum) = legacy_recency_checksum else {
        return Ok(());
    };
    if legacy_recency_checksum.as_slice() != recency_migration.checksum.as_ref()
        && !checksum_matches_eol_variant(recency_migration, legacy_recency_checksum.as_slice())
    {
        return Ok(());
    }

    sqlx::query(
        r#"
UPDATE _sqlx_migrations
SET version = ?, description = ?
WHERE version = ?
  AND checksum = ?
  AND NOT EXISTS (
      SELECT 1 FROM _sqlx_migrations WHERE version = ?
  )
        "#,
    )
    .bind(recency_migration.version)
    .bind(recency_migration.description.as_ref())
    .bind(38_i64)
    .bind(legacy_recency_checksum)
    .bind(recency_migration.version)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
#[path = "migrations_tests.rs"]
mod tests;
