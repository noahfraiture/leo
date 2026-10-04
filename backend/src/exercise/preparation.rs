//! Operator context and clarification history, separate from the authoritative checklist.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{Checklist, Error, Result};

/// Only the operator can choose whether list position imposes an assessment requirement.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChecklistOrder {
    #[default]
    Unspecified,
    ListedOrder,
    ExplicitOnly,
}

/// A question about a source criterion. An empty answer remains an unresolved policy question.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Clarification {
    pub criterion_id: Uuid,
    pub question: String,
    pub source_text: String,
    pub answer: String,
}

/// Reusable preparation inputs. Source text and questions never become a second grading checklist.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Preparation {
    pub scene_context: String,
    pub order: ChecklistOrder,
    /// Captured before the first wording proposal; later final edits do not alter this source.
    pub source: Option<Checklist>,
    pub questions: Vec<Clarification>,
    pub proposal: Option<super::ChecklistProposal>,
    pub last_review: Option<super::ChecklistProposal>,
    /// The source/context/answers that the operator last reconciled with the final wording.
    pub reviewed_basis: Option<String>,
}

impl Preparation {
    pub fn validate(&self, final_checklist: &Checklist) -> Result<()> {
        let source = self.source.as_ref().unwrap_or(final_checklist);
        let mut ids = std::collections::HashSet::new();
        if source
            .criteria
            .iter()
            .any(|item| item.id.is_nil() || !ids.insert(item.id))
        {
            return Err(Error::Invalid("source criterion IDs must be unique".into()));
        }
        if self.questions.iter().any(|question| {
            question.question.trim().is_empty() || !ids.contains(&question.criterion_id)
        }) {
            return Err(Error::Invalid(
                "clarification must name an original criterion".into(),
            ));
        }
        Ok(())
    }

    /// Mark an explicit operator review; nothing rewrites the final wording automatically.
    pub fn reconcile(&mut self) -> Result<()> {
        self.reviewed_basis = Some(self.basis()?);
        Ok(())
    }

    pub fn needs_reconciliation(&self) -> Result<bool> {
        let current = self.basis()?;
        Ok(self
            .reviewed_basis
            .as_ref()
            .is_some_and(|basis| *basis != current))
    }

    fn basis(&self) -> Result<String> {
        let bytes = serde_json::to_vec(&(
            &self.source,
            &self.scene_context,
            self.order,
            &self.questions,
        ))?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exercise::Criterion;

    #[test]
    fn changing_an_answer_requires_review_without_rewriting_the_source() {
        let criterion = Criterion {
            id: Uuid::new_v4(),
            text: "Place the item.".into(),
        };
        let mut preparation = Preparation {
            source: Some(Checklist {
                format_version: "operator-text-v1".into(),
                criteria: vec![criterion.clone()],
            }),
            questions: vec![Clarification {
                criterion_id: criterion.id,
                question: "Where?".into(),
                source_text: criterion.text.clone(),
                answer: String::new(),
            }],
            ..Default::default()
        };
        preparation.reconcile().unwrap();
        preparation.questions[0].answer = "On the table".into();
        assert!(preparation.needs_reconciliation().unwrap());
        assert_eq!(preparation.source.as_ref().unwrap().criteria[0], criterion);
        preparation.reconcile().unwrap();
        assert!(!preparation.needs_reconciliation().unwrap());
    }
}
