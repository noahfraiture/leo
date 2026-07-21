use std::net::SocketAddr;

use clap::Parser;

#[derive(Parser)]
#[command(version, about)]
pub struct Args {
    #[arg(short, long)]
    pub address: SocketAddr,
    #[arg(short, long = "camera", required = true)]
    pub cameras: Vec<SocketAddr>,
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

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
    }
}
