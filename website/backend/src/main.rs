#![allow(dead_code)]

mod analysis;
mod db;
mod http;
#[cfg(test)]
mod test;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let runtime = db::init_runtime().await?;
    http::router::run(runtime.db, runtime.upload_bucket_path).await?;
    Ok(())
}
