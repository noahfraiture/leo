use clap::Parser;
use std::net::SocketAddr;

#[derive(Clone, Parser)]
#[command(version, about)]
pub struct Args {
    #[arg(short, long)]
    pub address: SocketAddr,
}

pub fn parse_args() -> Args {
    let args = Args::parse();
    args
}
