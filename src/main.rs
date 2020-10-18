use std::env;
use std::fs;

use anyhow::Result;
use clap::{crate_authors, crate_version, Clap};

mod api;
mod cache;
mod config;

#[derive(Clap)]
struct Opts {
    #[clap(short, long)]
    config: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let opts: Opts = Opts::parse();

    let cfg: config::Config = toml::from_str(fs::read_to_string(opts.config)?.as_str())?;

    let api = api::API::new(cfg).await?;

    api.run().await
}
