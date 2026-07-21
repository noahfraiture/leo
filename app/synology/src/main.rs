mod api;
mod camera;
mod cli;
mod server;

use camera::Camera;
use clap::Parser;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let args = cli::Args::parse();
    let cameras = args
        .cameras
        .into_iter()
        .enumerate()
        .map(|(index, address)| Camera::new(index, address))
        .collect();
    server::start(cameras, args.address).await
}
