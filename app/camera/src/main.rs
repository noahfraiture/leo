mod camera;
mod cli;
mod error;
mod http;
mod rtsp;
mod vapix;

#[tokio::main]
async fn main() -> Result<(), error::Error> {
    run(cli::parse_args()).await
}

async fn run(args: cli::Args) -> Result<(), error::Error> {
    let cli::Args {
        address,
        rtsp_address,
        video,
    } = args;

    let camera = camera::Camera::new();
    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);
    let mut rtsp = tokio::select! {
        biased;
        result = &mut shutdown => return result.map_err(error::Error::ShutdownSignal),
        result = rtsp::Server::start(rtsp_address, video) => result?,
    };

    let result = tokio::select! {
        biased;
        result = &mut shutdown => result.map_err(error::Error::ShutdownSignal),
        result = http::serve(camera, address) => result.map_err(error::Error::Http),
        result = rtsp.wait() => result.map_err(error::Error::Rtsp),
    };

    match rtsp.stop().await {
        Ok(()) => result,
        Err(error) => Err(error.into()),
    }
}
