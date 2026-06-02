use sqlx::{postgres::PgPoolOptions, PgPool};

use crate::errors::GatewayError;

pub async fn connect(database_url: &str) -> Result<PgPool, GatewayError> {
    PgPoolOptions::new()
        .max_connections(10)
        .connect(database_url)
        .await
        .map_err(GatewayError::Database)
}

pub async fn migrate(pool: &PgPool) -> Result<(), GatewayError> {
    sqlx::migrate!("src/db/managed_agents/migrations")
        .run(pool)
        .await
        .map_err(GatewayError::Migration)
}
