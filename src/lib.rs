pub mod app;
pub mod config;
pub mod error;
pub mod models;
pub mod web;
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!();
