#![forbid(unsafe_code)]

mod agents;
mod assets;
mod config;
mod environments;
mod error;
mod hex;
mod markdown;
mod models;
mod plan_login;
mod projects;
mod providers;
mod responses;
mod sandbox;
mod security;
mod server;
mod sessions;
mod slices;
mod state;
mod storage;
mod template_filters;
mod tools;
mod vault;
mod workflows;

pub async fn run_server() -> Result<(), Box<dyn std::error::Error>> {
    server::run().await
}
