use std::{
    env, fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use surrealdb::{
    Surreal,
    engine::any::{self, Any},
    opt::{
        Config,
        capabilities::{Capabilities, ExperimentalFeature},
    },
};
use thiserror::Error;

use super::models::video::{Video, VideoError};

pub type Database = Surreal<Any>;

pub async fn init() -> Result<Database, DbInitError> {
    let config = DatabaseConfig::from_env()?;
    let local = config.local_paths()?;
    local.prepare()?;

    let db = any::connect((local.surreal_url.as_str(), surreal_config())).await?;
    bootstrap(
        &db,
        &config.surreal_namespace,
        &config.surreal_database,
        &local.upload_bucket_path,
    )
    .await?;
    Ok(db)
}

pub async fn bootstrap(
    db: &Database,
    namespace: &str,
    database: &str,
    upload_bucket_path: &Path,
) -> Result<(), VideoError> {
    db.use_ns(namespace).use_db(database).await?;
    Video::init(db, upload_bucket_path).await?;
    Ok(())
}

#[derive(Deserialize)]
struct DatabaseConfig {
    #[serde(default = "default_surreal_url")]
    surreal_url: String,
    #[serde(default = "default_surreal_namespace")]
    surreal_namespace: String,
    #[serde(default = "default_surreal_database")]
    surreal_database: String,
    #[serde(default = "default_upload_bucket_path")]
    surreal_upload_bucket_path: PathBuf,
}

impl DatabaseConfig {
    fn from_env() -> Result<Self, DbConfigError> {
        Ok(envy::from_env()?)
    }

    fn local_paths(&self) -> Result<ResolvedLocalPaths, DbConfigError> {
        let upload_bucket_path = resolve_repo_path(&self.surreal_upload_bucket_path);
        let surreal_url = normalize_local_surreal_url(&self.surreal_url)?;

        Ok(ResolvedLocalPaths {
            surreal_url,
            upload_bucket_path,
        })
    }
}

struct ResolvedLocalPaths {
    surreal_url: String,
    upload_bucket_path: PathBuf,
}

impl ResolvedLocalPaths {
    fn prepare(&self) -> Result<(), DbConfigError> {
        if let Some(path) = local_surreal_path(&self.surreal_url) {
            fs::create_dir_all(path)?;
        }

        fs::create_dir_all(&self.upload_bucket_path)?;

        Ok(())
    }
}

fn surreal_config() -> Config {
    let capabilities =
        Capabilities::new().with_experimental_feature_allowed(ExperimentalFeature::Files);

    Config::new().capabilities(capabilities)
}

fn normalize_local_surreal_url(value: &str) -> Result<String, DbConfigError> {
    let Some(path) = local_surreal_path(value) else {
        return Ok(value.to_owned());
    };

    let normalized = resolve_repo_path(path);
    Ok(format!("surrealkv://{}", normalized.display()))
}

fn local_surreal_path(value: &str) -> Option<&Path> {
    value
        .strip_prefix("surrealkv://")
        .map(Path::new)
        .filter(|path| !path.as_os_str().is_empty())
}

fn resolve_repo_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_owned();
    }

    repo_root().join(path)
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("backend crate should have a repository root parent")
        .to_owned()
}

fn default_surreal_url() -> String {
    "surrealkv://.data/surrealdb".to_owned()
}

fn default_surreal_namespace() -> String {
    "video_analysis".to_owned()
}

fn default_surreal_database() -> String {
    "app".to_owned()
}

fn default_upload_bucket_path() -> PathBuf {
    PathBuf::from(".data/uploads")
}

#[derive(Debug, Error)]
pub enum DbInitError {
    #[error(transparent)]
    Config(#[from] DbConfigError),
    #[error(transparent)]
    Video(#[from] VideoError),
    #[error(transparent)]
    Surreal(#[from] surrealdb::Error),
}

#[derive(Debug, Error)]
pub enum DbConfigError {
    #[error(transparent)]
    Env(#[from] envy::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
