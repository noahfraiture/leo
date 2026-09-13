use super::*;
use crate::model::{ModelProfile, ProviderSnapshot, Transport};

fn model(id: &str, memory_cost_mib: u32) -> ModelSnapshot {
    ModelSnapshot {
        profile: ModelProfile {
            id: id.into(),
            name: id.into(),
            provider_id: "provider".into(),
            model: id.into(),
            memory_cost_mib,
            image_limit: 16,
            max_output_tokens: None,
            reasoning_effort: None,
        },
        provider: ProviderSnapshot {
            id: "provider".into(),
            name: "Local server".into(),
            base_url: "http://localhost:1234/v1".into(),
            transport: Transport::ChatCompletions,
        },
    }
}

fn jobs(profiles: &[&str]) -> Vec<ScheduledJob> {
    profiles
        .iter()
        .enumerate()
        .map(|(index, id)| ScheduledJob {
            id: format!("job-{index}"),
            profile_id: (*id).into(),
        })
        .collect()
}

#[test]
fn memory_budget_allows_small_models_together_and_defers_the_large_model() {
    let models = [model("a", 12_000), model("b", 12_000), model("c", 20_000)];
    let waves = plan_execution(
        &jobs(&["a", "b", "c"]),
        &models,
        &ExecutionPolicy::default(),
    )
    .unwrap();
    assert_eq!(waves.len(), 2);
    assert_eq!(waves[0].job_ids, ["job-0", "job-1"]);
    assert_eq!(waves[0].memory_cost_mib, 24_000);
    assert_eq!(waves[1].retire_before.len(), 2);
}

#[test]
fn hosted_models_cost_zero_but_still_obey_concurrency_limits() {
    let policy = ExecutionPolicy {
        memory_budget_mib: 0,
        max_concurrent_jobs: 2,
        ..ExecutionPolicy::default()
    };
    let models = [model("a", 0), model("b", 0), model("c", 0)];
    let waves = plan_execution(&jobs(&["a", "b", "c"]), &models, &policy).unwrap();
    assert_eq!(waves.len(), 2);
    assert_eq!(waves[0].job_ids.len(), 2);
    assert_eq!(waves[0].memory_cost_mib, 0);
    assert!(waves[1].retire_before.is_empty());
}

#[test]
fn shared_physical_model_uses_one_conservative_memory_allowance() {
    let first = model("a", 20_000);
    let mut alias = model("alias", 24_000);
    alias.profile.model = first.profile.model.clone();
    alias.provider.base_url.push('/');
    alias.provider.transport = Transport::Responses;
    let policy = ExecutionPolicy {
        max_jobs_per_model: 2,
        ..ExecutionPolicy::default()
    };
    let waves = plan_execution(&jobs(&["a", "alias"]), &[first, alias], &policy).unwrap();
    assert_eq!(waves.len(), 1);
    assert_eq!(waves[0].memory_cost_mib, 24_000);
    assert_eq!(waves[0].resident_models.len(), 1);
}

#[test]
fn aliases_cannot_bypass_the_per_model_request_limit() {
    let first = model("a", 1000);
    let mut alias = model("alias", 1000);
    alias.profile.model = first.profile.model.clone();
    let waves = plan_execution(
        &jobs(&["a", "alias"]),
        &[first, alias],
        &ExecutionPolicy::default(),
    )
    .unwrap();
    assert_eq!(waves.len(), 2);
    assert!(waves[1].retire_before.is_empty());
}

#[test]
fn oversized_model_fails_before_scheduling_any_job() {
    let result = plan_execution(
        &jobs(&["too-big"]),
        &[model("too-big", 40_000)],
        &ExecutionPolicy::default(),
    );
    assert!(matches!(result, Err(Error::ModelExceedsMemoryBudget)));
}

#[test]
fn duplicate_job_identity_is_rejected() {
    let job = jobs(&["a"]).remove(0);
    assert!(matches!(
        plan_execution(
            &[job.clone(), job],
            &[model("a", 0)],
            &ExecutionPolicy::default()
        ),
        Err(Error::InvalidJobIdentity)
    ));
}

#[test]
fn execution_bounds_reject_zero_and_over_laptop_capacity() {
    for policy in [
        ExecutionPolicy {
            memory_budget_mib: 49_153,
            ..ExecutionPolicy::default()
        },
        ExecutionPolicy {
            max_concurrent_jobs: 0,
            ..ExecutionPolicy::default()
        },
        ExecutionPolicy {
            max_jobs_per_model: 0,
            ..ExecutionPolicy::default()
        },
        ExecutionPolicy {
            request_timeout_seconds: 0,
            ..ExecutionPolicy::default()
        },
    ] {
        assert!(matches!(
            policy.validate(),
            Err(Error::InvalidExecutionPolicy)
        ));
    }
}
