use std::{
    env, fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use surrealdb::engine::any;
use surrealdb::opt::{
    Config,
    capabilities::{Capabilities, ExperimentalFeature},
};

use crate::db::{self, Database, DbInitError};

pub struct TestDatabase {
    pub db: Database,
    pub upload_bucket_path: PathBuf,
}

pub async fn init() -> Result<Database, DbInitError> {
    Ok(init_with_bucket_path().await?.db)
}

pub async fn init_with_bucket_path() -> Result<TestDatabase, DbInitError> {
    let capabilities =
        Capabilities::new().with_experimental_feature_allowed(ExperimentalFeature::Files);
    let db = any::connect(("mem://", Config::new().capabilities(capabilities))).await?;
    let bucket_root = std::env::temp_dir().join("leo-website-test-uploads");
    fs::create_dir_all(&bucket_root)?;

    if env::var_os("SURREAL_BUCKET_FOLDER_ALLOWLIST").is_none() {
        unsafe {
            env::set_var(
                "SURREAL_BUCKET_FOLDER_ALLOWLIST",
                bucket_root.canonicalize()?,
            );
        }
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let bucket_path = bucket_root.join(format!("{}-{now}", std::process::id()));

    db::bootstrap(&db, "test", "test", &bucket_path).await?;
    Ok(TestDatabase {
        db,
        upload_bucket_path: bucket_path,
    })
}
