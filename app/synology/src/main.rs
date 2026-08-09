mod api;
mod camera;
mod cli;
mod recording;
mod server;

use camera::Camera;
use clap::Parser;

/// Loads simulator configuration and serves the fixture-backed Web API.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = cli::Args::parse();
    let mut cameras: Vec<_> = args
        .cameras
        .into_iter()
        .enumerate()
        .map(|(index, address)| Camera::new(index, address))
        .collect();
    if let Some(path) = args.recording_catalogue {
        recording::load_catalogue(&path, &mut cameras)?;
    }
    server::start(cameras, args.address).await?;
    Ok(())
}
