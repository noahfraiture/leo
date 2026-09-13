//! Checklist wording assistance validated by the bounded format comparison.
//! Responses remain source-linked proposals and never publish an exercise revision.

use std::collections::HashMap;

use rig_core::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::model::{Completion, JobRole, ModelClient, StructuredJob};

use super::{
    ChecklistProposal, Error, ExerciseDraft, ProposalComment, ProposalCommentKind,
    ProposedCriterion, Result, SourceRange,
};

/// Identifies the concise operator text format selected by the format-fidelity experiment.
pub const CHECKLIST_FORMAT_VERSION: &str = "leo-checklist-concise-v1";

const INSTRUCTIONS: &str = "You format an operator-owned exercise checklist. Do not evaluate a candidate or invent exercise policy. Preserve source language, every action, negation, quantity, condition, allowed order, repetition and retry rule. One output criterion per source criterion, retaining the exact source_id. Keep compound requirements together. Do not assume numbered steps impose strict order unless the source says so. Do not replace ambiguous wording by your preferred interpretation: preserve it and ask a comment question. Suggested policy changes, assumptions and questions belong only in comments, never in criterion text. No generic safety advice or extra domain instructions. Output a JSON object matching the supplied schema. No markdown fences.\nRewrite each source as concise action/outcome text in its existing language. No field labels or extra headings.";

#[derive(Serialize)]
struct SourceCriterion<'a> {
    id: Uuid,
    text: &'a str,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct FormattedChecklist {
    criteria: Vec<FormattedCriterion>,
    comments: Vec<FormattedComment>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(inline)]
struct FormattedCriterion {
    source_id: String,
    text: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(inline)]
struct FormattedComment {
    source_id: String,
    question: String,
}

/// Ask the selected model for wording and questions, bound to this exact captured draft.
/// The caller chooses the provider/profile and its token ceiling, and must revalidate freshness
/// before applying the response.
pub async fn format_checklist(
    draft: &ExerciseDraft,
    client: &ModelClient,
) -> Result<Completion<ChecklistProposal>> {
    let job = formatting_job(draft)?;
    let completion: Completion<FormattedChecklist> = client.complete(&job).await?;
    Ok(Completion {
        value: proposal_from_response(draft, completion.value)?,
        usage: completion.usage,
        elapsed_ms: completion.elapsed_ms,
    })
}

fn formatting_job(draft: &ExerciseDraft) -> Result<StructuredJob> {
    draft.validate()?;
    if draft.checklist.criteria.is_empty()
        || draft
            .checklist
            .criteria
            .iter()
            .any(|item| item.text.trim().is_empty())
    {
        return Err(Error::Invalid(
            "checklist formatting needs nonempty source criteria".into(),
        ));
    }
    let source = draft
        .checklist
        .criteria
        .iter()
        .map(|item| SourceCriterion {
            id: item.id,
            text: &item.text,
        })
        .collect::<Vec<_>>();
    let schema = rig_core::schemars::schema_for!(FormattedChecklist);
    Ok(StructuredJob {
        role: JobRole::ChecklistFormatter,
        instructions: format!(
            "{INSTRUCTIONS}\nSchema: {}",
            serde_json::to_string(&schema)?
        ),
        text: serde_json::to_string(&source)?,
        images: Vec::new(),
        max_output_tokens: None,
    })
}

fn proposal_from_response(
    draft: &ExerciseDraft,
    response: FormattedChecklist,
) -> Result<ChecklistProposal> {
    // Ranges refer to the exact source_text rendering, including its two-newline separators.
    let mut offset = 0;
    let sources = draft
        .checklist
        .criteria
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let range = SourceRange {
                start: offset,
                end: offset + item.text.len(),
            };
            offset = range.end + 2;
            (item.id, (index, range))
        })
        .collect::<HashMap<_, _>>();
    let mut returned = HashMap::new();
    for criterion in response.criteria {
        let id = parse_source_id(&criterion.source_id)?;
        if !sources.contains_key(&id) || returned.insert(id, criterion.text).is_some() {
            return Err(Error::Invalid(
                "formatter returned an unknown or repeated source criterion".into(),
            ));
        }
    }
    if returned.len() != sources.len() {
        return Err(Error::Invalid(
            "formatter omitted a source criterion; the original checklist is unchanged".into(),
        ));
    }
    // Provider ordering cannot reorder the operator's criteria or their reference links.
    let criteria = draft
        .checklist
        .criteria
        .iter()
        .map(|item| ProposedCriterion {
            text: returned.remove(&item.id).expect("source coverage checked"),
            source_ranges: vec![sources[&item.id].1.clone()],
        })
        .collect();
    let comments = response
        .comments
        .into_iter()
        .map(|comment| {
            let id = parse_source_id(&comment.source_id)?;
            let (index, range) = sources.get(&id).ok_or_else(|| {
                Error::Invalid("formatter comment names an unknown source criterion".into())
            })?;
            Ok(ProposalComment {
                kind: ProposalCommentKind::Ambiguity,
                text: comment.question,
                source_ranges: vec![range.clone()],
                proposed_item: Some(*index),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    ChecklistProposal::for_draft(draft, criteria, comments)
}

fn parse_source_id(value: &str) -> Result<Uuid> {
    Uuid::parse_str(value)
        .map_err(|_| Error::Invalid("formatter returned an invalid source criterion ID".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exercise::{Checklist, Criterion};

    fn draft() -> ExerciseDraft {
        ExerciseDraft {
            schema_version: 1,
            id: Uuid::new_v4(),
            name: "Set the table".into(),
            edit_version: 0,
            checklist: Checklist {
                format_version: CHECKLIST_FORMAT_VERSION.into(),
                criteria: vec![
                    Criterion {
                        id: Uuid::new_v4(),
                        text: "Déposer le bol.".into(),
                    },
                    Criterion {
                        id: Uuid::new_v4(),
                        text: "Poser la tasse.".into(),
                    },
                ],
            },
            references: Vec::new(),
        }
    }

    fn response(draft: &ExerciseDraft) -> FormattedChecklist {
        FormattedChecklist {
            criteria: draft
                .checklist
                .criteria
                .iter()
                .map(|item| FormattedCriterion {
                    source_id: item.id.to_string(),
                    text: item.text.clone(),
                })
                .collect(),
            comments: Vec::new(),
        }
    }

    #[test]
    fn provider_order_cannot_reorder_operator_items_or_comment_anchors() {
        let draft = draft();
        let mut output = response(&draft);
        output.criteria.reverse();
        output.comments.push(FormattedComment {
            source_id: draft.checklist.criteria[1].id.to_string(),
            question: "Sur quelle table ?".into(),
        });
        let proposal = proposal_from_response(&draft, output).unwrap();
        assert_eq!(proposal.criteria[0].text, draft.checklist.criteria[0].text);
        assert_eq!(proposal.criteria[1].text, draft.checklist.criteria[1].text);
        assert_eq!(proposal.comments[0].proposed_item, Some(1));
    }

    #[test]
    fn source_ranges_preserve_utf8_text_and_separators() {
        let draft = draft();
        let proposal = proposal_from_response(&draft, response(&draft)).unwrap();
        for (source, proposed) in draft.checklist.criteria.iter().zip(&proposal.criteria) {
            let range = &proposed.source_ranges[0];
            assert_eq!(&proposal.source_text[range.start..range.end], source.text);
        }
    }

    #[test]
    fn omitted_source_item_rejects_the_whole_proposal() {
        let draft = draft();
        let mut output = response(&draft);
        output.criteria.pop();
        assert!(proposal_from_response(&draft, output).is_err());
    }

    #[test]
    fn repeated_source_item_cannot_replace_a_missing_item() {
        let draft = draft();
        let mut output = response(&draft);
        output.criteria[1].source_id = output.criteria[0].source_id.clone();
        assert!(proposal_from_response(&draft, output).is_err());
    }

    #[test]
    fn invented_source_item_is_rejected() {
        let draft = draft();
        let mut output = response(&draft);
        output.criteria[0].source_id = Uuid::new_v4().to_string();
        assert!(proposal_from_response(&draft, output).is_err());
    }

    #[test]
    fn comment_cannot_target_an_unknown_source() {
        let draft = draft();
        let mut output = response(&draft);
        output.comments.push(FormattedComment {
            source_id: Uuid::new_v4().to_string(),
            question: "Where should this go?".into(),
        });
        assert!(proposal_from_response(&draft, output).is_err());
    }

    #[test]
    fn empty_rewrite_is_not_an_accepted_source_item() {
        let draft = draft();
        let mut output = response(&draft);
        output.criteria[0].text = " ".into();
        assert!(proposal_from_response(&draft, output).is_err());
    }

    #[test]
    fn blank_operator_item_is_rejected_before_a_model_request() {
        let mut draft = draft();
        draft.checklist.criteria[0].text.clear();
        assert!(formatting_job(&draft).is_err());
    }
}
