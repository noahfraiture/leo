use backend::model::Error;

#[test]
fn duplicate_selected_model_ids_are_rejected_instead_of_choosing_the_first_profile() {
    let mut workflow = crate::test_workflow(Some(crate::test_openai_config()), 8, 0);
    let mut duplicate = workflow.models[0].clone();
    duplicate.model = "different-model".into();
    workflow.models.push(duplicate);

    assert!(matches!(
        workflow.resolve_model("fixture"),
        Err(Error::UnknownModel)
    ));
}

#[test]
fn unrelated_incomplete_models_and_model_sets_do_not_block_the_selected_profile() {
    let mut workflow = crate::test_workflow(Some(crate::test_openai_config()), 8, 0);
    let mut incomplete = workflow.models[0].clone();
    incomplete.id = "unfinished".into();
    incomplete.model.clear();
    workflow.models.push(incomplete);
    workflow.model_sets[0].reader_ids.clear();

    assert!(workflow.resolve("fixture").is_err());
    assert_eq!(
        workflow.resolve_model("fixture").unwrap().profile.model,
        "test-model"
    );
    assert!(workflow.resolve_model("unfinished").is_err());
}

#[test]
fn selected_model_resolution_checks_its_memory_allowance() {
    let mut workflow = crate::test_workflow(Some(crate::test_openai_config()), 8, 0);
    workflow.execution.memory_budget_mib = 1024;
    workflow.models[0].memory_cost_mib = 2048;

    assert!(matches!(
        workflow.resolve_model("fixture"),
        Err(Error::ModelExceedsMemoryBudget)
    ));
    workflow.models[0].memory_cost_mib = 1024;
    assert!(workflow.resolve_model("fixture").is_ok());
}

#[test]
fn unusable_execution_policy_is_rejected_before_a_model_is_selected() {
    let mut workflow = crate::test_workflow(Some(crate::test_openai_config()), 8, 0);
    workflow.execution.request_timeout_seconds = 0;

    assert!(matches!(
        workflow.resolve_model("fixture"),
        Err(Error::InvalidExecutionPolicy)
    ));
}
