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
    db::bootstrap(
        &db,
        "test",
        "test",
        &std::env::temp_dir().join("leo-website-test-uploads"),
    )
    .await?;
    Ok(db)
}
