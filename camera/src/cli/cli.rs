use clap::Parser;
use std::{net::SocketAddr, path::PathBuf};

#[derive(Clone, Parser)]
#[command(version, about)]
pub(crate) struct Args {
    #[arg(short, long)]
    pub(crate) address: SocketAddr,
    #[arg(long)]
    pub(crate) rtsp_address: SocketAddr,
    #[arg(long)]
    pub(crate) video: PathBuf,
}

pub(crate) fn parse_args() -> Args {
    Args::parse()
}

#[cfg(test)]
mod tests {
    use std::{net::SocketAddr, path::PathBuf};

    use clap::Parser;

    use super::Args;

    #[test]
    fn parses_required_addresses_and_video() {
        let args = Args::try_parse_from([
            "camera",
            "--address",
            "127.0.0.1:8080",
            "--rtsp-address",
            "127.0.0.1:8554",
            "--video",
            "fixtures/video.mp4",
        ])
        .unwrap();

        assert_eq!(args.address, SocketAddr::from(([127, 0, 0, 1], 8080)));
        assert_eq!(args.rtsp_address, SocketAddr::from(([127, 0, 0, 1], 8554)));
        assert_eq!(args.video, PathBuf::from("fixtures/video.mp4"));
    }
}
