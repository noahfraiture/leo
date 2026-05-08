#![allow(dead_code)]

mod db;
mod http;
#[cfg(test)]
mod test;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db = db::init().await?;
    http::router::run(db).await?;
    Ok(())
}
