pub mod assets;
mod island_host;
mod generated {
    include!(concat!(env!("OUT_DIR"), "/generated_islands.rs"));
}
pub mod islands {
    pub use super::generated::*;
    pub use super::island_host::*;
}
pub mod props {
    tonic::include_proto!("props.v1");
}
