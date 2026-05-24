use surrealdb::types::{Datetime, RecordId, RecordIdKey, SurrealValue};
use thiserror::Error;

use crate::{
    analysis::{
        canary::DEFAULT_CANARY_PROMPT,
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
    pub failure_diagnostic: Option<AnalysisFailureDiagnostic>,
    pub is_canary: bool,
    pub history_hidden: bool,
    pub created_at: Datetime,
    pub updated_at: Datetime,
}

#[derive(Clone, Debug, PartialEq, SurrealValue)]
pub struct AnalysisFailureDiagnostic {
    pub stage: String,
    pub kind: String,
    pub retryable: bool,
    pub attempt: Option<i64>,
    pub attempts: Option<i64>,
    pub payload_bytes: Option<i64>,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, SurrealValue)]
pub struct AnalysisEvent {
    pub id: RecordId,
    pub analysis_key: String,
    pub provider: String,
    pub stage: String,
    pub level: String,
    pub message: String,
    pub attempt: Option<i64>,
    pub attempts: Option<i64>,
    pub payload_bytes: Option<i64>,
    pub offset_bytes: Option<i64>,
    pub size_bytes: Option<i64>,
    pub duration_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, SurrealValue)]
pub struct NewAnalysisEvent {
    pub analysis_key: String,
    pub provider: String,
    pub stage: String,
    pub level: String,
    pub message: String,
    pub attempt: Option<i64>,
    pub attempts: Option<i64>,
    pub payload_bytes: Option<i64>,
    pub offset_bytes: Option<i64>,
    pub size_bytes: Option<i64>,
    pub duration_ms: Option<i64>,
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
        define_analysis_table(db).await?;
        AnalysisEvent::init(db).await
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
        Self::create_record(db, provider, settings, prompt, video_keys, false).await
    }

    pub async fn create_canary_with_provider_and_settings(
        db: &Database,
        provider: AnalysisProvider,
        settings: AnalysisSettings,
        prompt: impl Into<String>,
        video_keys: Vec<String>,
    ) -> Result<Self, AnalysisError> {
        Self::create_record(db, provider, settings, prompt, video_keys, true).await
    }

    async fn create_record(
        db: &Database,
        provider: AnalysisProvider,
        settings: AnalysisSettings,
        prompt: impl Into<String>,
        video_keys: Vec<String>,
        is_canary: bool,
    ) -> Result<Self, AnalysisError> {
        #[derive(SurrealValue)]
        struct CreateAnalysis {
            provider: String,
            frame_sample_rate_fps: f64,
            prompt: String,
            video_keys: Vec<String>,
            is_canary: bool,
            history_hidden: bool,
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
                    failure_diagnostic: NONE,
                    is_canary: $is_canary,
                    history_hidden: $history_hidden,
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
                is_canary,
                history_hidden: false,
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
            .query(
                r#"
                SELECT * FROM analysis
                WHERE is_canary = false AND history_hidden = false
                ORDER BY created_at DESC
                LIMIT $limit START $offset;
                "#,
            )
            .bind(AnalysisPage {
                limit: limit as i64,
                offset: offset as i64,
            })
            .await?;

        Ok(response.take(0)?)
    }

    pub async fn clear_history(db: &Database) -> Result<usize, AnalysisError> {
        let mut response = db
            .query(
                r#"
                SELECT * FROM analysis
                WHERE is_canary = false AND history_hidden = false;
                UPDATE analysis MERGE {
                    history_hidden: true,
                    updated_at: time::now(),
                }
                WHERE is_canary = false AND history_hidden = false;
                "#,
            )
            .await?;
        let visible: Vec<Analysis> = response.take(0)?;
        response.check()?;

        Ok(visible.len())
    }

    pub async fn hide_from_history(
        db: &Database,
        key: &str,
    ) -> Result<Option<Self>, AnalysisError> {
        let mut response = db
            .query(
                r#"
                UPDATE $id MERGE {
                    history_hidden: true,
                    updated_at: time::now(),
                }
                WHERE is_canary = false;
                "#,
            )
            .bind(AnalysisId {
                id: analysis_id(key),
            })
            .await?;

        let mut updated: Vec<Analysis> = response.take(0)?;
        Ok(updated.pop())
    }

    pub async fn delete_canaries_for_provider(
        db: &Database,
        provider: AnalysisProvider,
    ) -> Result<usize, AnalysisError> {
        #[derive(SurrealValue)]
        struct CanaryProvider {
            provider: String,
        }

        let mut response = db
            .query(
                r#"
                SELECT * FROM analysis
                WHERE is_canary = true AND provider = $provider;
                "#,
            )
            .bind(CanaryProvider {
                provider: provider.to_string(),
            })
            .await?;
        let canaries: Vec<Analysis> = response.take(0)?;
        let deleted = canaries.len();

        for analysis in canaries {
            AnalysisEvent::delete_for_analysis(db, &analysis.key()).await?;
            db.query("DELETE $id;")
                .bind(AnalysisId { id: analysis.id })
                .await?
                .check()?;
        }

        Ok(deleted)
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
                failure_diagnostic: NONE,
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
                failure_diagnostic: NONE,
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

    pub async fn fail_with_diagnostic(
        &self,
        db: &Database,
        diagnostic: AnalysisFailureDiagnostic,
    ) -> Result<(), AnalysisError> {
        #[derive(SurrealValue)]
        struct FailAnalysis {
            id: RecordId,
            error: String,
            diagnostic: AnalysisFailureDiagnostic,
        }

        db.query(
            r#"
            UPDATE $id MERGE {
                status: "failed",
                response: NONE,
                error: $error,
                failure_diagnostic: $diagnostic,
                updated_at: time::now(),
            };
            "#,
        )
        .bind(FailAnalysis {
            id: self.id.clone(),
            error: diagnostic.message.clone(),
            diagnostic,
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

impl AnalysisEvent {
    pub async fn init(db: &Database) -> Result<(), AnalysisError> {
        define_analysis_event_table(db).await
    }

    pub async fn record(db: &Database, event: NewAnalysisEvent) -> Result<(), AnalysisError> {
        db.query(
            r#"
            CREATE analysis_event CONTENT {
                analysis_key: $analysis_key,
                provider: $provider,
                stage: $stage,
                level: $level,
                message: $message,
                attempt: $attempt,
                attempts: $attempts,
                payload_bytes: $payload_bytes,
                offset_bytes: $offset_bytes,
                size_bytes: $size_bytes,
                duration_ms: $duration_ms,
                created_at: time::now(),
            };
            "#,
        )
        .bind(event)
        .await?
        .check()?;

        Ok(())
    }

    pub async fn list_for_analysis(
        db: &Database,
        analysis_key: &str,
    ) -> Result<Vec<Self>, AnalysisError> {
        #[derive(SurrealValue)]
        struct AnalysisEvents {
            analysis_key: String,
        }

        let mut response = db
            .query(
                r#"
                SELECT * FROM analysis_event
                WHERE analysis_key = $analysis_key
                ORDER BY created_at ASC;
                "#,
            )
            .bind(AnalysisEvents {
                analysis_key: analysis_key.to_owned(),
            })
            .await?;

        Ok(response.take(0)?)
    }

    pub async fn delete_for_analysis(
        db: &Database,
        analysis_key: &str,
    ) -> Result<(), AnalysisError> {
        #[derive(SurrealValue)]
        struct AnalysisEvents {
            analysis_key: String,
        }

        db.query("DELETE analysis_event WHERE analysis_key = $analysis_key;")
            .bind(AnalysisEvents {
                analysis_key: analysis_key.to_owned(),
            })
            .await?
            .check()?;

        Ok(())
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
        DEFINE FIELD IF NOT EXISTS failure_diagnostic ON TABLE analysis TYPE option<object>;
        DEFINE FIELD IF NOT EXISTS failure_diagnostic.stage ON TABLE analysis TYPE option<string>;
        DEFINE FIELD IF NOT EXISTS failure_diagnostic.kind ON TABLE analysis TYPE option<string>;
        DEFINE FIELD IF NOT EXISTS failure_diagnostic.retryable ON TABLE analysis TYPE option<bool>;
        DEFINE FIELD IF NOT EXISTS failure_diagnostic.attempt ON TABLE analysis TYPE option<int>;
        DEFINE FIELD IF NOT EXISTS failure_diagnostic.attempts ON TABLE analysis TYPE option<int>;
        DEFINE FIELD IF NOT EXISTS failure_diagnostic.payload_bytes ON TABLE analysis TYPE option<int>;
        DEFINE FIELD IF NOT EXISTS failure_diagnostic.message ON TABLE analysis TYPE option<string>;
        DEFINE FIELD IF NOT EXISTS is_canary ON TABLE analysis TYPE bool DEFAULT false;
        DEFINE FIELD IF NOT EXISTS history_hidden ON TABLE analysis TYPE bool DEFAULT false;
        DEFINE FIELD IF NOT EXISTS created_at ON TABLE analysis TYPE datetime;
        DEFINE FIELD IF NOT EXISTS updated_at ON TABLE analysis TYPE datetime;
        UPDATE analysis MERGE { provider: "gemini" } WHERE provider = NONE;
        UPDATE analysis MERGE { frame_sample_rate_fps: $default_frame_sample_rate_fps } WHERE frame_sample_rate_fps = NONE;
        UPDATE analysis MERGE { failure_diagnostic: NONE } WHERE failure_diagnostic = NONE;
        UPDATE analysis MERGE { is_canary: false } WHERE is_canary = NONE;
        UPDATE analysis MERGE { history_hidden: false } WHERE history_hidden = NONE;
        UPDATE analysis MERGE { is_canary: true } WHERE prompt = $default_canary_prompt;
        "#,
    )
    .bind(("default_frame_sample_rate_fps", DEFAULT_FRAME_SAMPLE_RATE_FPS))
    .bind(("default_canary_prompt", DEFAULT_CANARY_PROMPT))
    .await?
    .check()?;

    Ok(())
}

async fn define_analysis_event_table(db: &Database) -> Result<(), AnalysisError> {
    db.query(
        r#"
        DEFINE TABLE IF NOT EXISTS analysis_event SCHEMAFULL;
        DEFINE FIELD IF NOT EXISTS analysis_key ON TABLE analysis_event TYPE string;
        DEFINE FIELD IF NOT EXISTS provider ON TABLE analysis_event TYPE string;
        DEFINE FIELD IF NOT EXISTS stage ON TABLE analysis_event TYPE string;
        DEFINE FIELD IF NOT EXISTS level ON TABLE analysis_event TYPE string ASSERT $value IN ["info", "warn", "error"];
        DEFINE FIELD IF NOT EXISTS message ON TABLE analysis_event TYPE string;
        DEFINE FIELD IF NOT EXISTS attempt ON TABLE analysis_event TYPE option<int>;
        DEFINE FIELD IF NOT EXISTS attempts ON TABLE analysis_event TYPE option<int>;
        DEFINE FIELD IF NOT EXISTS payload_bytes ON TABLE analysis_event TYPE option<int>;
        DEFINE FIELD IF NOT EXISTS offset_bytes ON TABLE analysis_event TYPE option<int>;
        DEFINE FIELD IF NOT EXISTS size_bytes ON TABLE analysis_event TYPE option<int>;
        DEFINE FIELD IF NOT EXISTS duration_ms ON TABLE analysis_event TYPE option<int>;
        DEFINE FIELD IF NOT EXISTS created_at ON TABLE analysis_event TYPE datetime;
        DEFINE INDEX IF NOT EXISTS analysis_event_analysis_created ON TABLE analysis_event FIELDS analysis_key, created_at;
        "#,
    )
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

    #[tokio::test]
    async fn list_page_excludes_canary_analyses() {
        let database = crate::test::database::init()
            .await
            .expect("test database should initialize");

        let visible =
            db::analysis::Analysis::create(&database, "User prompt", vec!["user.mp4".to_owned()])
                .await
                .expect("visible analysis should create");
        let hidden = db::analysis::Analysis::create_canary_with_provider_and_settings(
            &database,
            AnalysisProvider::OpenAi,
            crate::analysis::request::AnalysisSettings {
                frame_sample_rate_fps: 1.0,
            },
            "Canary prompt",
            vec!["canary.mp4".to_owned()],
        )
        .await
        .expect("canary analysis should create");

        let page = db::analysis::Analysis::list_page(&database, 10, 0)
            .await
            .expect("analysis page should list");
        let found_hidden = db::analysis::Analysis::find(&database, &hidden.key())
            .await
            .expect("canary should load directly")
            .expect("canary should exist");

        assert_eq!(page.len(), 1);
        assert_eq!(page[0].key(), visible.key());
        assert!(found_hidden.is_canary);
    }

    #[tokio::test]
    async fn clear_history_hides_regular_analyses_without_deleting_records() {
        let database = crate::test::database::init()
            .await
            .expect("test database should initialize");

        let visible =
            db::analysis::Analysis::create(&database, "User prompt", vec!["user.mp4".to_owned()])
                .await
                .expect("visible analysis should create");
        let canary = db::analysis::Analysis::create_canary_with_provider_and_settings(
            &database,
            AnalysisProvider::OpenAi,
            crate::analysis::request::AnalysisSettings {
                frame_sample_rate_fps: 1.0,
            },
            "Canary prompt",
            vec!["canary.mp4".to_owned()],
        )
        .await
        .expect("canary analysis should create");

        let hidden = db::analysis::Analysis::clear_history(&database)
            .await
            .expect("analysis history should clear");

        let page = db::analysis::Analysis::list_page(&database, 10, 0)
            .await
            .expect("analysis page should list");
        let found_visible = db::analysis::Analysis::find(&database, &visible.key())
            .await
            .expect("visible analysis should load directly")
            .expect("visible analysis should still exist");
        let found_canary = db::analysis::Analysis::find(&database, &canary.key())
            .await
            .expect("canary should load directly")
            .expect("canary should still exist");

        assert_eq!(hidden, 1);
        assert!(page.is_empty());
        assert!(found_visible.history_hidden);
        assert!(!found_canary.history_hidden);
    }

    #[tokio::test]
    async fn hide_from_history_hides_one_regular_analysis_without_deleting_record() {
        let database = crate::test::database::init()
            .await
            .expect("test database should initialize");

        let hidden =
            db::analysis::Analysis::create(&database, "Hide me", vec!["hide.mp4".to_owned()])
                .await
                .expect("hidden analysis should create");
        let visible =
            db::analysis::Analysis::create(&database, "Keep me", vec!["keep.mp4".to_owned()])
                .await
                .expect("visible analysis should create");
        let canary = db::analysis::Analysis::create_canary_with_provider_and_settings(
            &database,
            AnalysisProvider::OpenAi,
            crate::analysis::request::AnalysisSettings {
                frame_sample_rate_fps: 1.0,
            },
            "Canary prompt",
            vec!["canary.mp4".to_owned()],
        )
        .await
        .expect("canary analysis should create");

        let updated = db::analysis::Analysis::hide_from_history(&database, &hidden.key())
            .await
            .expect("analysis should hide from history")
            .expect("analysis should be updated");
        let canary_update = db::analysis::Analysis::hide_from_history(&database, &canary.key())
            .await
            .expect("canary hide should be ignored");

        let page = db::analysis::Analysis::list_page(&database, 10, 0)
            .await
            .expect("analysis page should list");
        let found_hidden = db::analysis::Analysis::find(&database, &hidden.key())
            .await
            .expect("hidden analysis should load directly")
            .expect("hidden analysis should still exist");
        let found_canary = db::analysis::Analysis::find(&database, &canary.key())
            .await
            .expect("canary should load directly")
            .expect("canary should still exist");

        assert_eq!(updated.key(), hidden.key());
        assert!(updated.history_hidden);
        assert!(canary_update.is_none());
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].key(), visible.key());
        assert!(found_hidden.history_hidden);
        assert!(!found_canary.history_hidden);
    }

    #[tokio::test]
    async fn delete_canaries_for_provider_removes_only_matching_canary_events() {
        let database = crate::test::database::init()
            .await
            .expect("test database should initialize");
        let regular = db::analysis::Analysis::create_with_provider(
            &database,
            AnalysisProvider::OpenAi,
            "Regular prompt",
            vec!["regular.mp4".to_owned()],
        )
        .await
        .expect("regular analysis should create");
        let openai_canary = db::analysis::Analysis::create_canary_with_provider_and_settings(
            &database,
            AnalysisProvider::OpenAi,
            crate::analysis::request::AnalysisSettings {
                frame_sample_rate_fps: 1.0,
            },
            "OpenAI canary",
            vec!["canary.mp4".to_owned()],
        )
        .await
        .expect("openai canary should create");
        let gemini_canary = db::analysis::Analysis::create_canary_with_provider_and_settings(
            &database,
            AnalysisProvider::Gemini,
            crate::analysis::request::AnalysisSettings {
                frame_sample_rate_fps: 1.0,
            },
            "Gemini canary",
            vec!["canary.mp4".to_owned()],
        )
        .await
        .expect("gemini canary should create");
        db::analysis::AnalysisEvent::record(
            &database,
            db::analysis::NewAnalysisEvent {
                analysis_key: openai_canary.key(),
                provider: "openai".to_owned(),
                stage: "queued".to_owned(),
                level: "info".to_owned(),
                message: "canary queued".to_owned(),
                attempt: None,
                attempts: None,
                payload_bytes: None,
                offset_bytes: None,
                size_bytes: None,
                duration_ms: None,
            },
        )
        .await
        .expect("event should record");

        let deleted = db::analysis::Analysis::delete_canaries_for_provider(
            &database,
            AnalysisProvider::OpenAi,
        )
        .await
        .expect("openai canaries should delete");

        assert_eq!(deleted, 1);
        assert!(
            db::analysis::Analysis::find(&database, &openai_canary.key())
                .await
                .expect("openai canary lookup should complete")
                .is_none()
        );
        assert!(
            db::analysis::AnalysisEvent::list_for_analysis(&database, &openai_canary.key())
                .await
                .expect("openai canary events should list")
                .is_empty()
        );
        assert!(
            db::analysis::Analysis::find(&database, &regular.key())
                .await
                .expect("regular lookup should complete")
                .is_some()
        );
        assert!(
            db::analysis::Analysis::find(&database, &gemini_canary.key())
                .await
                .expect("gemini canary lookup should complete")
                .is_some()
        );
    }

    #[tokio::test]
    async fn fail_with_diagnostic_persists_sanitized_failure_fields() {
        let database = crate::test::database::init()
            .await
            .expect("test database should initialize");
        let analysis =
            db::analysis::Analysis::create(&database, "Check the video", vec!["sample.mp4".into()])
                .await
                .expect("analysis should create");

        analysis
            .fail_with_diagnostic(
                &database,
                db::analysis::AnalysisFailureDiagnostic {
                    stage: "openai.chunk".to_owned(),
                    kind: "timeout".to_owned(),
                    retryable: true,
                    attempt: Some(3),
                    attempts: Some(3),
                    payload_bytes: Some(1_234_567),
                    message: "request timed out".to_owned(),
                },
            )
            .await
            .expect("analysis should fail with diagnostics");

        let found = db::analysis::Analysis::find(&database, &analysis.key())
            .await
            .expect("analysis should load")
            .expect("analysis should exist");

        assert_eq!(found.status, "failed");
        assert_eq!(found.error.as_deref(), Some("request timed out"));
        assert_eq!(
            found.failure_diagnostic,
            Some(db::analysis::AnalysisFailureDiagnostic {
                stage: "openai.chunk".to_owned(),
                kind: "timeout".to_owned(),
                retryable: true,
                attempt: Some(3),
                attempts: Some(3),
                payload_bytes: Some(1_234_567),
                message: "request timed out".to_owned(),
            })
        );
    }

    #[tokio::test]
    async fn analysis_events_are_listed_oldest_first_for_one_analysis() {
        let database = crate::test::database::init()
            .await
            .expect("test database should initialize");
        let first = db::analysis::Analysis::create(&database, "First", vec!["first.mp4".into()])
            .await
            .expect("first analysis should create");
        let second = db::analysis::Analysis::create(&database, "Second", vec!["second.mp4".into()])
            .await
            .expect("second analysis should create");

        db::analysis::AnalysisEvent::record(
            &database,
            db::analysis::NewAnalysisEvent {
                analysis_key: first.key(),
                provider: "gemini".to_owned(),
                stage: "queued".to_owned(),
                level: "info".to_owned(),
                message: "analysis queued".to_owned(),
                attempt: None,
                attempts: None,
                payload_bytes: None,
                offset_bytes: None,
                size_bytes: None,
                duration_ms: None,
            },
        )
        .await
        .expect("event should record");
        db::analysis::AnalysisEvent::record(
            &database,
            db::analysis::NewAnalysisEvent {
                analysis_key: second.key(),
                provider: "openai".to_owned(),
                stage: "queued".to_owned(),
                level: "info".to_owned(),
                message: "other analysis queued".to_owned(),
                attempt: None,
                attempts: None,
                payload_bytes: None,
                offset_bytes: None,
                size_bytes: None,
                duration_ms: None,
            },
        )
        .await
        .expect("event should record");
        db::analysis::AnalysisEvent::record(
            &database,
            db::analysis::NewAnalysisEvent {
                analysis_key: first.key(),
                provider: "gemini".to_owned(),
                stage: "complete".to_owned(),
                level: "info".to_owned(),
                message: "analysis completed".to_owned(),
                attempt: None,
                attempts: None,
                payload_bytes: None,
                offset_bytes: None,
                size_bytes: None,
                duration_ms: Some(42),
            },
        )
        .await
        .expect("event should record");

        let events = db::analysis::AnalysisEvent::list_for_analysis(&database, &first.key())
            .await
            .expect("events should list");

        assert_eq!(
            events
                .iter()
                .map(|event| event.stage.as_str())
                .collect::<Vec<_>>(),
            vec!["queued", "complete"]
        );
        assert_eq!(events[1].duration_ms, Some(42));
    }
}
