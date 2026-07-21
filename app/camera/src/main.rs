mod camera;
mod cli;
mod server;
mod vapix;

use camera::Camera;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let args = cli::parse_args();
    let camera = Camera::new();
    server::start(camera, args.address).await
}
