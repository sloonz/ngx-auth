use anyhow::{Result, bail};
use sqlx::{AnyPool, Row, any::AnyPoolOptions};
use std::env;

#[derive(Clone)]
pub struct Database {
    pub pool: AnyPool,
    mysql: bool,
}

impl Database {
    pub async fn from_env() -> Result<Self> {
        let kind = env::var("DB_TYPE").unwrap_or_else(|_| "sqlite3".into());
        let mysql = match kind.as_str() {
            "sqlite3" => false,
            "mysql2" => true,
            _ => bail!("unsupported DB_TYPE: {kind}"),
        };
        let url = if mysql {
            let mut url = url::Url::parse("mysql://localhost")?;
            url.set_host(Some(
                &env::var("DB_HOST").unwrap_or_else(|_| "localhost".into()),
            ))?;
            url.set_username(&env::var("DB_USER").unwrap_or_default())
                .map_err(|_| anyhow::anyhow!("invalid DB_USER"))?;
            url.set_password(Some(&env::var("DB_PASSWORD").unwrap_or_default()))
                .map_err(|_| anyhow::anyhow!("invalid DB_PASSWORD"))?;
            url.set_path(&format!("/{}", env::var("DB_NAME").unwrap_or_default()));
            if let Ok(socket) = env::var("DB_SOCKET_PATH").or_else(|e| {
                if env::var_os("DB_HOST").is_none() {
                    Ok("/run/mysqld/mysqld.sock".into())
                } else {
                    Err(e)
                }
            }) {
                url.query_pairs_mut().append_pair("socket", &socket);
            }
            url.to_string()
        } else {
            let filename = env::var("DB_FILENAME").unwrap_or_else(|_| "db.sqlite".into());
            let mut url = url::Url::parse("sqlite://localhost")?;
            url.set_host(None)
                .map_err(|_| anyhow::anyhow!("invalid SQLite URL"))?;
            // Absolute paths also avoid URL interpretation of relative filenames.
            let path = std::path::absolute(filename)?;
            url.set_path(
                path.to_str()
                    .ok_or_else(|| anyhow::anyhow!("non-UTF8 database path"))?,
            );
            url.query_pairs_mut().append_pair("mode", "rwc");
            url.to_string()
        };
        Self::connect(&url, mysql).await
    }

    pub async fn connect(url: &str, mysql: bool) -> Result<Self> {
        sqlx::any::install_default_drivers();
        let pool = AnyPoolOptions::new()
            .max_connections(if mysql { 5 } else { 1 })
            .connect(url)
            .await?;
        let db = Self { pool, mysql };
        super::migrations::run(&db.pool, mysql).await?;
        Ok(db)
    }

    pub async fn cleanup(&self) -> Result<()> {
        let sql = if self.mysql {
            "DELETE FROM sessions WHERE expiration_date <= UTC_TIMESTAMP()"
        } else {
            "DELETE FROM sessions WHERE CASE WHEN typeof(expiration_date) IN ('integer', 'real') THEN expiration_date <= CAST(strftime('%s', 'now') AS INTEGER) * 1000 ELSE julianday(expiration_date) <= julianday('now') END"
        };
        sqlx::query(sql).execute(&self.pool).await?;
        Ok(())
    }
    pub async fn session(&self, id: &str) -> Result<Option<Option<i64>>> {
        Ok(
            sqlx::query("SELECT CAST(user_id AS SIGNED) AS user_id FROM sessions WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?
                .map(|r| r.try_get("user_id"))
                .transpose()?,
        )
    }
    pub async fn authorized(&self, user: i64, origin: &str) -> Result<bool> {
        Ok(sqlx::query("SELECT 1 FROM authorizations a JOIN origins o ON a.origin_id = o.id WHERE a.user_id = ? AND o.origin = ? LIMIT 1")
            .bind(user).bind(origin).fetch_optional(&self.pool).await?.is_some())
    }
    pub async fn policy(&self, origin: &str) -> Result<String> {
        let row =
            sqlx::query("SELECT same_site_cookie_policy FROM origins WHERE origin = ? LIMIT 1")
                .bind(origin)
                .fetch_optional(&self.pool)
                .await?;
        let policy = row
            .and_then(|r| r.try_get::<Option<String>, _>(0).ok().flatten())
            .unwrap_or_else(|| "lax".into());
        Ok(match policy.as_str() {
            "none" => "None",
            "strict" => "Strict",
            _ => "Lax",
        }
        .into())
    }
    pub async fn user(&self, email: &str) -> Result<i64> {
        // Keep user lookup and creation in one transaction.
        let mut tx = self.pool.begin().await?;
        let find = "SELECT CAST(id AS SIGNED) AS id FROM users WHERE email = ? LIMIT 1";
        let row = sqlx::query(find)
            .bind(email)
            .fetch_optional(&mut *tx)
            .await?;
        let id = if let Some(row) = row {
            row.try_get("id")?
        } else {
            sqlx::query("INSERT INTO users (email) VALUES (?)")
                .bind(email)
                .execute(&mut *tx)
                .await?;
            sqlx::query(find)
                .bind(email)
                .fetch_one(&mut *tx)
                .await?
                .try_get("id")?
        };
        tx.commit().await?;
        Ok(id)
    }
    pub async fn login(&self, session: &str, user: i64) -> Result<()> {
        let sql = if self.mysql {
            "INSERT INTO sessions (id, user_id, expiration_date) VALUES (?, ?, DATE_ADD(UTC_TIMESTAMP(), INTERVAL 1 DAY))"
        } else {
            "INSERT INTO sessions (id, user_id, expiration_date) VALUES (?, ?, (CAST(strftime('%s', 'now') AS INTEGER) + 86400) * 1000)"
        };
        sqlx::query(sql)
            .bind(session)
            .bind(user)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
