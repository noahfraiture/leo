mod camera;
mod cli;
mod server;
mod vapix;

use camera::Camera;

fn main() {
    let args = cli::parse_args();
    let camera = Camera::new();
    server::start(camera, args.address);
}
