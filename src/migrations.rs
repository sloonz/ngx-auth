use anyhow::{Result, ensure};
use sqlx::{
    AnyConnection, AnyPool, Connection,
    migrate::{Migrate, Migrator},
};

pub async fn run(pool: &AnyPool, mysql: bool) -> Result<()> {
    let mut connection = pool.acquire().await?;
    let connection = &mut *connection;
    let migrator = if mysql {
        sqlx::migrate!("migrations/mysql")
    } else {
        sqlx::migrate!("migrations/sqlite")
    };
    let has_history = table_exists(connection, mysql, "_sqlx_migrations").await?
        && sqlx::query("SELECT 1 FROM _sqlx_migrations LIMIT 1")
            .fetch_optional(&mut *connection)
            .await?
            .is_some();
    if !has_history {
        let existing = knex_version(connection, mysql).await?;
        connection.ensure_migrations_table().await?;
        if let Some(version) = existing {
            record_baseline(connection, &migrator, version).await?;
        }
    }
    migrator.run(&mut *connection).await?;
    Ok(())
}

async fn record_baseline(
    connection: &mut AnyConnection,
    migrator: &Migrator,
    version: i64,
) -> Result<()> {
    let mut transaction = connection.begin().await?;
    for migration in migrator.iter().filter(|m| m.version <= version) {
        // Preserve the embedded migration's checksum, but execute no DDL: the
        // matching schema already exists. SQLx writes its own migration history.
        let mut adopted = migration.clone();
        adopted.sql = "SELECT 1;".into();
        transaction.apply(&adopted).await?;
    }
    transaction.commit().await?;
    Ok(())
}

async fn table_exists(connection: &mut AnyConnection, mysql: bool, table: &str) -> Result<bool> {
    let sql = if mysql {
        "SELECT 1 FROM information_schema.tables WHERE table_schema = DATABASE() AND table_name = ?"
    } else {
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?"
    };
    Ok(sqlx::query(sql)
        .bind(table)
        .fetch_optional(connection)
        .await?
        .is_some())
}

async fn knex_version(connection: &mut AnyConnection, mysql: bool) -> Result<Option<i64>> {
    if !table_exists(connection, mysql, "knex_migrations").await? {
        return Ok(None);
    }
    let names: Vec<String> = sqlx::query_scalar("SELECT name FROM knex_migrations ORDER BY id")
        .fetch_all(connection)
        .await?;
    const KNEX_MIGRATIONS: &[&str] = &["initial-tables", "origin-add-same-site-cookie-policy"];
    ensure!(
        names.len() <= KNEX_MIGRATIONS.len() && names == KNEX_MIGRATIONS[..names.len()],
        "Unrecognized Knex migration history: {names:?}"
    );
    Ok((!names.is_empty()).then_some(names.len() as i64))
}
