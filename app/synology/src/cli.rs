use std::{net::SocketAddr, path::PathBuf};

use clap::Parser;

/// Command-line configuration for the Synology simulator process.
#[derive(Parser)]
#[command(version, about)]
pub struct Args {
    /// Address on which the simulator serves its Web API.
    #[arg(short, long)]
    pub address: SocketAddr,
    /// Camera HTTP addresses, whose order assigns their IDs.
    #[arg(short, long = "camera", required = true)]
    pub cameras: Vec<SocketAddr>,
    /// Optional fixture catalogue loaded before the server starts.
    #[arg(long)]
    pub recording_catalogue: Option<PathBuf>,
}

#[cfg(test)]
mod tests {
    use std::{
        net::{IpAddr, Ipv4Addr, SocketAddr},
        path::PathBuf,
    };

    use clap::Parser;

    use super::Args;

    #[test]
    fn parses_address_and_cameras() {
        let args = Args::try_parse_from([
            "synology",
            "--address",
            "127.0.0.1:5000",
            "--camera",
            "127.0.0.1:8001",
            "--camera",
            "127.0.0.1:8002",
        ])
        .unwrap();

        assert_eq!(
            args.address,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 5000)
        );
        assert_eq!(
            args.cameras,
            [
                SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8001),
                SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8002),
            ]
        );
        assert_eq!(args.recording_catalogue, None);
    }

    #[test]
    fn parses_recording_catalogue() {
        let args = Args::try_parse_from([
            "synology",
            "--address",
            "127.0.0.1:5000",
            "--camera",
            "127.0.0.1:8001",
            "--recording-catalogue",
            "fixtures/recordings.json",
        ])
        .unwrap();

        assert_eq!(
            args.recording_catalogue,
            Some(PathBuf::from("fixtures/recordings.json"))
        );
    }
}
