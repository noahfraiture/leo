mod cli;

use std::{env, io};

use camera::{camera::Camera, server};

#[tokio::main]
async fn main() -> io::Result<()> {
    let address = cli::parse(env::args_os().skip(1))?;
    server::start(Camera::new(), address).await
}
