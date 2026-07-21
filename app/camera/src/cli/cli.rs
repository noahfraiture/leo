use clap::Parser;
use std::net::SocketAddr;

#[derive(Clone, Parser)]
#[command(version, about)]
pub(crate) struct Args {
    #[arg(short, long)]
    pub(crate) address: SocketAddr,
}

pub(crate) fn parse_args() -> Args {
    Args::parse()
}
