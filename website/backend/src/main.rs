#![allow(dead_code)]

mod db;
mod grpc;
mod http;
#[cfg(test)]
mod test;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db = db::init().await?;
    tokio::try_join!(http::router::run(db), grpc::run())?;
    Ok(())
}
