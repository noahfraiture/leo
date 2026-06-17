#![allow(dead_code)]

mod analysis;
mod app;
mod canary;
mod cli;
mod db;
mod http;
mod media;
#[cfg(test)]
mod test;
mod upload;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    match cli::Cli::parse_args()
        .command
        .unwrap_or(cli::Command::Serve)
    {
        cli::Command::Serve => {
            let runtime = db::init_runtime().await?;
            http::router::run(runtime.db, runtime.upload_bucket_path).await?;
        }
        cli::Command::Analyze(args) => cli::analyze(args).await?,
    }

    Ok(())
}
