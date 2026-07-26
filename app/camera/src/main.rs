mod camera;
mod cli;
mod http;
mod vapix;

use camera::Camera;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let args = cli::parse_args();
    let camera = Camera::new();
    http::serve(camera, args.address).await
}
