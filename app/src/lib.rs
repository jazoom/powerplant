#![forbid(unsafe_code)]

mod assets;
mod config;
mod error;
mod markdown;
mod providers;
mod responses;
mod security;
mod server;
mod sessions;
mod slices;
mod state;
mod template_filters;
mod vault;

pub async fn run_server() -> Result<(), Box<dyn std::error::Error>> {
    server::run().await
}
