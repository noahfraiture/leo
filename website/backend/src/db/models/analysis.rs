use surrealdb::types::{RecordId, RecordIdKey, SurrealValue};
use thiserror::Error;

use crate::db::Database;

const ANALYSIS_TABLE: &str = "analysis";

#[derive(Clone, Debug, PartialEq, SurrealValue)]
pub struct Analysis {
    pub id: RecordId,
    pub status: String,
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

impl Analysis {
    pub async fn init(db: &Database) -> Result<(), AnalysisError> {
        define_analysis_table(db).await
    }

    pub async fn create(
        db: &Database,
        prompt: impl Into<String>,
        video_keys: Vec<String>,
    ) -> Result<Self, AnalysisError> {
        #[derive(SurrealValue)]
        struct CreateAnalysis {
            prompt: String,
            video_keys: Vec<String>,
        }

        let mut response = db
            .query(
                r#"
                CREATE analysis CONTENT {
                    status: "queued",
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
