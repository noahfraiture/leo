use std::{
    env, fs,
    time::{SystemTime, UNIX_EPOCH},
};
use surrealdb::engine::any;
use surrealdb::opt::{
    Config,
    capabilities::{Capabilities, ExperimentalFeature},
};

use crate::db::{self, Database, video::VideoError};

pub async fn init() -> Result<Database, VideoError> {
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
    Ok(db)
}
