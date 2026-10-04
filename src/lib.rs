pub mod app;
pub mod cli;
pub mod config;
pub mod discord;
pub mod error;
pub mod models;
pub mod storage;
pub mod web;
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!();
