use dioxus::prelude::*;

use super::super::state::{SettingsContext, SettingsField, SettingsPageState};

/// Numeric input with persistent unfinished text, inline validation, and no silent fallback value.
#[component]
pub fn NumberField(
    name: &'static str,
    label: &'static str,
    value: Option<u64>,
    minimum: u64,
    maximum: u64,
    onvalid: EventHandler<Option<u64>>,
    model_id: Option<String>,
    #[props(default = false)] optional: bool,
) -> Element {
    let mut state = use_context::<SettingsContext>().state;
    let field = SettingsField::WorkflowNumber {
        model_id: model_id.clone(),
        name,
    };
    let (text, error) = {
        let page = state.read();
        (
            page.number_inputs
                .get(&field)
                .cloned()
                .unwrap_or_else(|| value.map(|number| number.to_string()).unwrap_or_default()),
            page.field_errors.get(&field).cloned(),
        )
    };
    let id = format!(
        "workflow-{}-{name}",
        model_id.as_deref().unwrap_or("policy")
    );
    let error_id = format!("{id}-error");
    rsx! {
        div { class: "flex flex-col gap-1",
            label { r#for: id.clone(), "{label}" }
            input {
                id,
                class: "input input-bordered w-full",
                r#type: "text",
                inputmode: "numeric",
                value: text,
                placeholder: if optional { "Provider default" } else { "" },
                aria_invalid: error.as_ref().map(|_| "true"),
                aria_describedby: error.as_ref().map(|_| error_id.clone()),
                oninput: move |event| {
                    let parsed = record_input(
                        &mut state.write(),
                        field.clone(),
                        event.value(),
                        minimum,
                        maximum,
                        optional,
                    );
                    if let Ok(value) = parsed {
                        onvalid.call(value);
                    }
                },
            }
            if let Some(ref error) = error {
                p {
                    id: error_id.clone(),
                    class: "text-error text-sm",
                    role: "alert",
                    "{error}"
                }
            }
        }
    }
}

fn record_input(
    page: &mut SettingsPageState,
    field: SettingsField,
    text: String,
    minimum: u64,
    maximum: u64,
    optional: bool,
) -> Result<Option<u64>, ()> {
    let trimmed = text.trim();
    let parsed = if trimmed.is_empty() && optional {
        Ok(None)
    } else {
        // Explicitly reject signs, decimals and exponent notation instead of coercing them.
        trimmed
            .parse::<u64>()
            .ok()
            .filter(|_| !trimmed.is_empty() && trimmed.bytes().all(|byte| byte.is_ascii_digit()))
            .filter(|number| (minimum..=maximum).contains(number))
            .map(Some)
            .ok_or(())
    };
    page.number_inputs.insert(field.clone(), text);
    match parsed {
        Ok(_) => {
            page.field_errors.remove(&field);
        }
        Err(()) => {
            let range = if maximum == u64::MAX {
                format!("Enter a whole number of at least {minimum}.")
            } else {
                format!("Enter a whole number from {minimum} to {maximum}.")
            };
            let message = if optional {
                format!("{range} Leave blank to use the provider default.")
            } else {
                range
            };
            page.field_errors.insert(field, message);
        }
    }
    page.save_error = None;
    page.restart_required = false;
    parsed
}

#[cfg(test)]
mod tests {
    use super::*;
    use backend::model::ModelProfile;

    #[test]
    fn invalid_input_cannot_submit_the_previous_numeric_value() {
        let mut page = SettingsPageState::new(crate::settings::Settings::default());
        let field = SettingsField::WorkflowNumber {
            model_id: None,
            name: "memory",
        };
        let previous = page.draft.workflow.execution.memory_budget_mib;
        for invalid in ["", "-1", "1.5", "49153", "1e3"] {
            assert!(
                record_input(&mut page, field.clone(), invalid.into(), 0, 49152, false).is_err()
            );
            assert_eq!(page.number_inputs[&field], invalid);
            assert_eq!(page.draft.workflow.execution.memory_budget_mib, previous);
            assert!(matches!(page.submission(), Err(errors) if errors.contains_key(&field)));
        }
        assert_eq!(
            record_input(&mut page, field, "0".into(), 0, 49152, false),
            Ok(Some(0))
        );
        assert!(page.submission().is_ok());
    }

    #[test]
    fn removing_an_invalid_model_restores_submission() {
        let mut page = SettingsPageState::new(crate::settings::Settings::default());
        page.draft.workflow.models.push(ModelProfile {
            id: "test".into(),
            name: "Test".into(),
            provider_id: "local".into(),
            model: "test".into(),
            memory_cost_mib: 0,
            image_limit: 16,
            max_output_tokens: None,
            reasoning_effort: Some("low".into()),
        });
        let field = SettingsField::WorkflowNumber {
            model_id: Some("test".into()),
            name: "images",
        };
        assert!(record_input(&mut page, field, "0".into(), 1, u64::MAX, false).is_err());
        assert!(page.submission().is_err());
        page.remove_workflow_model("test");
        assert!(page.submission().is_ok());
        assert!(page.number_inputs.is_empty());
        assert!(page.field_errors.is_empty());
    }

    #[test]
    fn optional_output_tokens_distinguish_blank_from_invalid_text() {
        let mut page = SettingsPageState::new(crate::settings::Settings::default());
        let field = SettingsField::WorkflowNumber {
            model_id: None,
            name: "tokens",
        };
        assert_eq!(
            record_input(&mut page, field.clone(), "".into(), 1, u64::MAX, true),
            Ok(None)
        );
        assert!(record_input(&mut page, field.clone(), "0".into(), 1, u64::MAX, true).is_err());
        assert_eq!(
            record_input(&mut page, field, "6000".into(), 1, u64::MAX, true),
            Ok(Some(6000))
        );
    }
}
