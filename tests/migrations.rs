use ngx_auth::database::Database;
use sqlx::{AnyPool, Row, any::AnyPoolOptions};

const LEGACY_SESSION: &str = "0123456789abcdef0123456789abcdef";

async fn sqlite_pool(url: &str) -> AnyPool {
    sqlx::any::install_default_drivers();
    AnyPoolOptions::new()
        .max_connections(1)
        .connect(url)
        .await
        .unwrap()
}

async fn assert_baseline(pool: &AnyPool) {
    let versions: Vec<i64> =
        sqlx::query_scalar("SELECT version FROM _sqlx_migrations ORDER BY version")
            .fetch_all(pool)
            .await
            .unwrap();
    assert_eq!(versions, [1, 2]);
    let dirty = sqlx::query("SELECT 1 FROM _sqlx_migrations WHERE success = FALSE")
        .fetch_all(pool)
        .await
        .unwrap();
    assert!(dirty.is_empty());
}

async fn assert_legacy_data(db: &Database) {
    assert_eq!(db.user("user@example.com").await.unwrap(), 1);
    assert_eq!(
        db.policy("https://protected.example").await.unwrap(),
        "Strict"
    );
    assert!(db.authorized(1, "https://protected.example").await.unwrap());
    db.cleanup().await.unwrap();
    assert_eq!(db.session(LEGACY_SESSION).await.unwrap(), Some(Some(1)));
    let names: Vec<String> = sqlx::query_scalar("SELECT name FROM knex_migrations ORDER BY id")
        .fetch_all(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        names,
        ["initial-tables", "origin-add-same-site-cookie-policy"]
    );
}

async fn sqlite_schema(pool: &AnyPool) -> Vec<(String, String)> {
    sqlx::query_as("SELECT name, sql FROM sqlite_master WHERE sql IS NOT NULL AND name <> '_sqlx_migrations' ORDER BY name").fetch_all(pool).await.unwrap()
}

#[tokio::test]
async fn adopts_actual_knex_sqlite_database_without_changing_schema_or_data() {
    let directory =
        std::env::temp_dir().join(format!("ngx-auth-migrations-{}", rand::random::<u64>()));
    std::fs::create_dir(&directory).unwrap();
    let url = format!("sqlite://{}/legacy.sqlite?mode=rwc", directory.display());
    let old = sqlite_pool(&url).await;
    sqlx::raw_sql(include_str!("fixtures/knex-sqlite.sql"))
        .execute(&old)
        .await
        .unwrap();
    let schema = sqlite_schema(&old).await;
    let expiration: i64 =
        sqlx::query_scalar("SELECT CAST(expiration_date AS SIGNED) FROM sessions")
            .fetch_one(&old)
            .await
            .unwrap();
    let history: Vec<(i64, String, i64, i64)> = sqlx::query_as(
        "SELECT id, name, batch, CAST(migration_time AS SIGNED) FROM knex_migrations ORDER BY id",
    )
    .fetch_all(&old)
    .await
    .unwrap();
    // Exercise first startup and a subsequent restart.
    for _ in 0..2 {
        let db = Database::connect(&url, false).await.unwrap();
        assert_baseline(&db.pool).await;
        assert_legacy_data(&db).await;
        assert_eq!(sqlite_schema(&db.pool).await, schema);
        let after: i64 = sqlx::query_scalar("SELECT CAST(expiration_date AS SIGNED) FROM sessions")
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(expiration, after);
        let after: Vec<(i64, String, i64, i64)> = sqlx::query_as(
            "SELECT id, name, batch, CAST(migration_time AS SIGNED) FROM knex_migrations ORDER BY id",
        )
        .fetch_all(&db.pool)
        .await
        .unwrap();
        assert_eq!(history, after);
        db.pool.close().await;
    }
    old.close().await;
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test]
async fn creates_fresh_schema_and_detects_changed_migration_checksums() {
    let filename =
        std::env::temp_dir().join(format!("ngx-auth-fresh-{}.sqlite", rand::random::<u64>()));
    let url = format!("sqlite://{}?mode=rwc", filename.display());
    let db = Database::connect(&url, false).await.unwrap();
    assert_baseline(&db.pool).await;
    let columns = sqlx::query(
        "SELECT 1 FROM pragma_table_info('origins') WHERE name = 'same_site_cookie_policy'",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(columns.len(), 1);
    // A restart must use SQLx's checksum validation, not adopt the schema again.
    sqlx::query("UPDATE _sqlx_migrations SET checksum = X'00' WHERE version = 1")
        .execute(&db.pool)
        .await
        .unwrap();
    let error = Database::connect(&url, false).await.err().unwrap();
    assert!(
        error
            .to_string()
            .contains("previously applied but has been modified"),
        "{error:#}"
    );
    db.pool.close().await;
    std::fs::remove_file(filename).unwrap();
}

#[tokio::test]
async fn rejects_unrecognized_knex_migration_history() {
    for statement in [
        "UPDATE knex_migrations SET name = 'unknown-migration' WHERE id = 2",
        "DELETE FROM knex_migrations WHERE id = 1",
        "INSERT INTO knex_migrations (name, batch) VALUES ('initial-tables', 2)",
    ] {
        let filename =
            std::env::temp_dir().join(format!("ngx-auth-invalid-{}.sqlite", rand::random::<u64>()));
        let url = format!("sqlite://{}?mode=rwc", filename.display());
        let old = sqlite_pool(&url).await;
        sqlx::raw_sql(include_str!("fixtures/knex-sqlite.sql"))
            .execute(&old)
            .await
            .unwrap();
        sqlx::query(statement).execute(&old).await.unwrap();
        let schema = sqlite_schema(&old).await;
        let error = Database::connect(&url, false).await.err().unwrap();
        assert!(
            error
                .to_string()
                .contains("Unrecognized Knex migration history"),
            "{statement}: {error:#}"
        );
        assert_eq!(sqlite_schema(&old).await, schema);
        assert!(
            sqlx::query("SELECT 1 FROM sqlite_master WHERE name = '_sqlx_migrations'")
                .fetch_optional(&old)
                .await
                .unwrap()
                .is_none()
        );
        old.close().await;
        std::fs::remove_file(filename).unwrap();
    }
}

async fn mysql_schema(pool: &AnyPool) -> Vec<String> {
    let mut schema = Vec::new();
    for table in [
        "users",
        "origins",
        "authorizations",
        "sessions",
        "knex_migrations",
        "knex_migrations_lock",
    ] {
        let row = sqlx::query(&format!("SHOW CREATE TABLE {table}"))
            .fetch_one(pool)
            .await
            .unwrap();
        schema.push(row.try_get(1).unwrap());
    }
    schema
}

// Requires an account allowed to create/drop disposable databases. For example:
// NGX_AUTH_TEST_MYSQL_URL='mysql://root@localhost/?socket=/tmp/mysql.sock' \
//     cargo test --test migrations -- --ignored
#[tokio::test]
#[ignore = "requires NGX_AUTH_TEST_MYSQL_URL pointing to a MySQL/MariaDB server"]
async fn mysql_fresh_install_knex_adoption_and_restart() {
    sqlx::any::install_default_drivers();
    let base = std::env::var("NGX_AUTH_TEST_MYSQL_URL").expect("set NGX_AUTH_TEST_MYSQL_URL");
    let admin = AnyPoolOptions::new()
        .max_connections(1)
        .connect(&base)
        .await
        .unwrap();
    let name = format!("ngx_auth_test_{}", rand::random::<u64>());
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    let mut url = url::Url::parse(&base).unwrap();
    url.set_path(&format!("/{name}"));
    let url = url.to_string();
    let old = AnyPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!("fixtures/knex-mysql.sql"))
        .execute(&old)
        .await
        .unwrap();
    let schema = mysql_schema(&old).await;
    let expiration: String =
        sqlx::query_scalar("SELECT CAST(expiration_date AS CHAR) FROM sessions")
            .fetch_one(&old)
            .await
            .unwrap();
    let history: Vec<(i64, String, i64, String)> = sqlx::query_as("SELECT CAST(id AS SIGNED), name, CAST(batch AS SIGNED), CAST(migration_time AS CHAR) FROM knex_migrations ORDER BY id").fetch_all(&old).await.unwrap();
    for _ in 0..2 {
        let db = Database::connect(&url, true).await.unwrap();
        assert_baseline(&db.pool).await;
        assert_legacy_data(&db).await;
        assert_eq!(mysql_schema(&db.pool).await, schema);
        let after: String =
            sqlx::query_scalar("SELECT CAST(expiration_date AS CHAR) FROM sessions")
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert_eq!(expiration, after);
        let after: Vec<(i64, String, i64, String)> = sqlx::query_as("SELECT CAST(id AS SIGNED), name, CAST(batch AS SIGNED), CAST(migration_time AS CHAR) FROM knex_migrations ORDER BY id").fetch_all(&db.pool).await.unwrap();
        assert_eq!(history, after);
        db.pool.close().await;
    }
    // The existing SQLx history is authoritative on subsequent starts.
    sqlx::query("UPDATE _sqlx_migrations SET checksum = X'00' WHERE version = 1")
        .execute(&old)
        .await
        .unwrap();
    assert!(Database::connect(&url, true).await.is_err());
    old.close().await;
    sqlx::query(&format!("DROP DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    let fresh = Database::connect(&url, true).await.unwrap();
    assert_baseline(&fresh.pool).await;
    let user = fresh.user("new@example.com").await.unwrap();
    fresh.login(LEGACY_SESSION, user).await.unwrap();
    assert_eq!(
        fresh.session(LEGACY_SESSION).await.unwrap(),
        Some(Some(user))
    );
    fresh.pool.close().await;
    let restart = Database::connect(&url, true).await.unwrap();
    assert_baseline(&restart.pool).await;
    restart.pool.close().await;
    sqlx::query(&format!("DROP DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    // Unrecognized Knex histories must fail before creating SQLx history.
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    let old = AnyPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!("fixtures/knex-mysql.sql"))
        .execute(&old)
        .await
        .unwrap();
    sqlx::query("UPDATE knex_migrations SET name = 'unknown-migration' WHERE id = 2")
        .execute(&old)
        .await
        .unwrap();
    assert!(Database::connect(&url, true).await.is_err());
    assert!(sqlx::query("SELECT 1 FROM information_schema.tables WHERE table_schema = DATABASE() AND table_name = '_sqlx_migrations'").fetch_optional(&old).await.unwrap().is_none());
    sqlx::query(
        "UPDATE knex_migrations SET name = 'origin-add-same-site-cookie-policy' WHERE id = 2",
    )
    .execute(&old)
    .await
    .unwrap();
    // Also exercise applying a pending migration to an older Knex database.
    sqlx::query("ALTER TABLE origins DROP COLUMN same_site_cookie_policy")
        .execute(&old)
        .await
        .unwrap();
    sqlx::query("DELETE FROM knex_migrations WHERE id = 2")
        .execute(&old)
        .await
        .unwrap();
    let repaired = Database::connect(&url, true).await.unwrap();
    assert_baseline(&repaired.pool).await;
    assert_eq!(
        repaired.policy("https://protected.example").await.unwrap(),
        "Lax"
    );
    assert_eq!(
        repaired.session(LEGACY_SESSION).await.unwrap(),
        Some(Some(1))
    );
    repaired.pool.close().await;
    old.close().await;
    sqlx::query(&format!("DROP DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
}

#[tokio::test]
async fn adopts_first_knex_version_and_applies_pending_cookie_policy_migration() {
    let filename =
        std::env::temp_dir().join(format!("ngx-auth-older-{}.sqlite", rand::random::<u64>()));
    let url = format!("sqlite://{}?mode=rwc", filename.display());
    let old = sqlite_pool(&url).await;
    sqlx::raw_sql(include_str!("fixtures/knex-sqlite.sql"))
        .execute(&old)
        .await
        .unwrap();
    sqlx::query("ALTER TABLE origins DROP COLUMN same_site_cookie_policy")
        .execute(&old)
        .await
        .unwrap();
    sqlx::query("DELETE FROM knex_migrations WHERE id = 2")
        .execute(&old)
        .await
        .unwrap();
    let db = Database::connect(&url, false).await.unwrap();
    assert_baseline(&db.pool).await;
    assert_eq!(db.policy("https://protected.example").await.unwrap(), "Lax");
    assert_eq!(db.session(LEGACY_SESSION).await.unwrap(), Some(Some(1)));
    let names: Vec<String> = sqlx::query_scalar("SELECT name FROM knex_migrations")
        .fetch_all(&db.pool)
        .await
        .unwrap();
    assert_eq!(names, ["initial-tables"]);
    db.pool.close().await;
    old.close().await;
    std::fs::remove_file(filename).unwrap();
}
