use surrealdb::types::{RecordId, RecordIdKey, SurrealValue};
use thiserror::Error;

use crate::{
    analysis::{
        provider::AnalysisProvider,
        request::{AnalysisSettings, DEFAULT_FRAME_SAMPLE_RATE_FPS},
    },
    db::Database,
};

const ANALYSIS_TABLE: &str = "analysis";

#[derive(Clone, Debug, PartialEq, SurrealValue)]
pub struct Analysis {
    pub id: RecordId,
    pub status: String,
    pub provider: String,
    pub frame_sample_rate_fps: f64,
    pub prompt: String,
    pub video_keys: Vec<String>,
    pub response: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Error)]
pub enum AnalysisError {
    #[error("database did not return the created analysis record")]
    MissingCreatedRecord,
    #[error(transparent)]
    Surreal(#[from] surrealdb::Error),
}

#[derive(SurrealValue)]
struct AnalysisId {
    id: RecordId,
}

#[derive(SurrealValue)]
struct AnalysisPage {
    limit: i64,
    offset: i64,
}

impl Analysis {
    pub async fn init(db: &Database) -> Result<(), AnalysisError> {
        define_analysis_table(db).await
    }

    pub async fn create(
        db: &Database,
        prompt: impl Into<String>,
        video_keys: Vec<String>,
    ) -> Result<Self, AnalysisError> {
        Self::create_with_provider(db, AnalysisProvider::Gemini, prompt, video_keys).await
    }

    pub async fn create_with_provider(
        db: &Database,
        provider: AnalysisProvider,
        prompt: impl Into<String>,
        video_keys: Vec<String>,
    ) -> Result<Self, AnalysisError> {
        Self::create_with_provider_and_settings(
            db,
            provider,
            AnalysisSettings::default(),
            prompt,
            video_keys,
        )
        .await
    }

    pub async fn create_with_provider_and_settings(
        db: &Database,
        provider: AnalysisProvider,
        settings: AnalysisSettings,
        prompt: impl Into<String>,
        video_keys: Vec<String>,
    ) -> Result<Self, AnalysisError> {
        #[derive(SurrealValue)]
        struct CreateAnalysis {
            provider: String,
            frame_sample_rate_fps: f64,
            prompt: String,
            video_keys: Vec<String>,
        }

        let mut response = db
            .query(
                r#"
                CREATE analysis CONTENT {
                    status: "queued",
                    provider: $provider,
                    frame_sample_rate_fps: $frame_sample_rate_fps,
                    prompt: $prompt,
                    video_keys: $video_keys,
                    response: NONE,
                    error: NONE,
                    created_at: time::now(),
                    updated_at: time::now(),
                };
                "#,
            )
            .bind(CreateAnalysis {
                provider: provider.to_string(),
                frame_sample_rate_fps: settings.frame_sample_rate_fps,
                prompt: prompt.into(),
                video_keys,
            })
            .await?;

        let mut created: Vec<Analysis> = response.take(0)?;
        created.pop().ok_or(AnalysisError::MissingCreatedRecord)
    }

    pub async fn find(db: &Database, key: &str) -> Result<Option<Self>, AnalysisError> {
        let mut response = db
            .query("SELECT * FROM $id;")
            .bind(AnalysisId {
                id: analysis_id(key),
            })
            .await?;

        let mut analyses: Vec<Analysis> = response.take(0)?;
        Ok(analyses.pop())
    }

    pub async fn list_recent(db: &Database, limit: usize) -> Result<Vec<Self>, AnalysisError> {
        Self::list_page(db, limit, 0).await
    }

    pub async fn list_page(
        db: &Database,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Self>, AnalysisError> {
        let mut response = db
            .query("SELECT * FROM analysis ORDER BY created_at DESC LIMIT $limit START $offset;")
            .bind(AnalysisPage {
                limit: limit as i64,
                offset: offset as i64,
            })
            .await?;

        Ok(response.take(0)?)
    }

    pub async fn mark_running(&self, db: &Database) -> Result<(), AnalysisError> {
        db.query(
            r#"
            UPDATE $id MERGE {
                status: "running",
                updated_at: time::now(),
            };
            "#,
        )
        .bind(AnalysisId {
            id: self.id.clone(),
        })
        .await?
        .check()?;

        Ok(())
    }

    pub async fn complete(
        &self,
        db: &Database,
        response: impl Into<String>,
    ) -> Result<(), AnalysisError> {
        #[derive(SurrealValue)]
        struct CompleteAnalysis {
            id: RecordId,
            response: String,
        }

        db.query(
            r#"
            UPDATE $id MERGE {
                status: "complete",
                response: $response,
                error: NONE,
                updated_at: time::now(),
            };
            "#,
        )
        .bind(CompleteAnalysis {
            id: self.id.clone(),
            response: response.into(),
        })
        .await?
        .check()?;

        Ok(())
    }

    pub async fn fail(&self, db: &Database, error: impl Into<String>) -> Result<(), AnalysisError> {
        #[derive(SurrealValue)]
        struct FailAnalysis {
            id: RecordId,
            error: String,
        }

        db.query(
            r#"
            UPDATE $id MERGE {
                status: "failed",
                response: NONE,
                error: $error,
                updated_at: time::now(),
            };
            "#,
        )
        .bind(FailAnalysis {
            id: self.id.clone(),
            error: error.into(),
        })
        .await?
        .check()?;

        Ok(())
    }

    pub fn key(&self) -> String {
        match &self.id.key {
            RecordIdKey::String(key) => key.clone(),
            RecordIdKey::Number(key) => key.to_string(),
            _ => String::new(),
        }
    }

    pub fn is_pending(&self) -> bool {
        matches!(self.status.as_str(), "queued" | "running")
    }
}

async fn define_analysis_table(db: &Database) -> Result<(), AnalysisError> {
    db.query(
        r#"
        DEFINE TABLE IF NOT EXISTS analysis SCHEMAFULL;
        DEFINE FIELD IF NOT EXISTS status ON TABLE analysis TYPE string ASSERT $value IN ["queued", "running", "complete", "failed"];
        DEFINE FIELD IF NOT EXISTS provider ON TABLE analysis TYPE string DEFAULT "gemini" ASSERT $value IN ["gemini", "openai"];
        DEFINE FIELD IF NOT EXISTS frame_sample_rate_fps ON TABLE analysis TYPE float DEFAULT 0.2 ASSERT $value > 0;
        DEFINE FIELD IF NOT EXISTS prompt ON TABLE analysis TYPE string;
        DEFINE FIELD IF NOT EXISTS video_keys ON TABLE analysis TYPE array<string>;
        DEFINE FIELD IF NOT EXISTS response ON TABLE analysis TYPE option<string>;
        DEFINE FIELD IF NOT EXISTS error ON TABLE analysis TYPE option<string>;
        DEFINE FIELD IF NOT EXISTS created_at ON TABLE analysis TYPE datetime;
        DEFINE FIELD IF NOT EXISTS updated_at ON TABLE analysis TYPE datetime;
        UPDATE analysis MERGE { provider: "gemini" } WHERE provider = NONE;
        UPDATE analysis MERGE { frame_sample_rate_fps: $default_frame_sample_rate_fps } WHERE frame_sample_rate_fps = NONE;
        "#,
    )
    .bind(("default_frame_sample_rate_fps", DEFAULT_FRAME_SAMPLE_RATE_FPS))
    .await?
    .check()?;

    Ok(())
}

fn analysis_id(key: &str) -> RecordId {
    RecordId::new(ANALYSIS_TABLE, key)
}

#[cfg(test)]
mod tests {
    use crate::{analysis::provider::AnalysisProvider, db};

    #[tokio::test]
    async fn create_with_provider_persists_selected_provider() {
        let database = crate::test::database::init()
            .await
            .expect("test database should initialize");

        let analysis = db::analysis::Analysis::create_with_provider(
            &database,
            AnalysisProvider::OpenAi,
            "Summarize the video",
            vec!["sample.mp4".to_owned()],
        )
        .await
        .expect("analysis should create");
        let found = db::analysis::Analysis::find(&database, &analysis.key())
            .await
            .expect("analysis should load")
            .expect("analysis should exist");

        assert_eq!(found.provider, "openai");
    }

    #[tokio::test]
    async fn create_with_settings_persists_selected_frame_sample_rate() {
        let database = crate::test::database::init()
            .await
            .expect("test database should initialize");

        let analysis = db::analysis::Analysis::create_with_provider_and_settings(
            &database,
            AnalysisProvider::OpenAi,
            crate::analysis::request::AnalysisSettings {
                frame_sample_rate_fps: 2.0,
            },
            "Summarize the video",
            vec!["sample.mp4".to_owned()],
        )
        .await
        .expect("analysis should create");
        let found = db::analysis::Analysis::find(&database, &analysis.key())
            .await
            .expect("analysis should load")
            .expect("analysis should exist");

        assert_eq!(found.frame_sample_rate_fps, 2.0);
    }

    #[tokio::test]
    async fn list_page_returns_newest_analyses_with_limit_and_offset() {
        let database = crate::test::database::init()
            .await
            .expect("test database should initialize");

        let first =
            db::analysis::Analysis::create(&database, "First prompt", vec!["first.mp4".to_owned()])
                .await
                .expect("first analysis should create");
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        let second = db::analysis::Analysis::create(
            &database,
            "Second prompt",
            vec!["second.mp4".to_owned()],
        )
        .await
        .expect("second analysis should create");
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        let third =
            db::analysis::Analysis::create(&database, "Third prompt", vec!["third.mp4".to_owned()])
                .await
                .expect("third analysis should create");

        let page = db::analysis::Analysis::list_page(&database, 2, 1)
            .await
            .expect("analysis page should list");

        assert_eq!(
            page.iter()
                .map(db::analysis::Analysis::key)
                .collect::<Vec<_>>(),
            vec![second.key(), first.key()]
        );
        assert!(!page.iter().any(|analysis| analysis.key() == third.key()));
    }
}
