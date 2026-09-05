use super::{
    AnalysisProfile, ImageSizePolicy, MonitoringProfile, validate_analysis_profiles,
    validate_monitoring_profiles,
};

#[test]
fn profile_validation_covers_identity_limits_and_subsecond_sampling() {
    let base = MonitoringProfile {
        id: 1,
        name: "Motion".into(),
        sample_every_ms: 500,
    };
    validate_monitoring_profiles(std::slice::from_ref(&base)).unwrap();
    for invalid in [
        MonitoringProfile {
            id: 0,
            ..base.clone()
        },
        MonitoringProfile {
            name: " ".into(),
            ..base.clone()
        },
        MonitoringProfile {
            sample_every_ms: 0,
            ..base.clone()
        },
    ] {
        assert!(validate_monitoring_profiles(&[invalid]).is_err());
    }
    assert!(validate_monitoring_profiles(&[base.clone(), base.clone()]).is_err());
    assert!(
        validate_monitoring_profiles(&[base.clone(), MonitoringProfile { id: 2, ..base }]).is_err()
    );

    let base = crate::tests::analysis_profile(8, 2);
    validate_analysis_profiles(std::slice::from_ref(&base)).unwrap();
    for invalid in [
        AnalysisProfile {
            model: " ".into(),
            ..base.clone()
        },
        AnalysisProfile {
            max_images_per_prompt: 0,
            ..base.clone()
        },
        AnalysisProfile {
            max_prompt_span_ms: 0,
            ..base.clone()
        },
        AnalysisProfile {
            overlap_frame_sets: 8,
            ..base.clone()
        },
        AnalysisProfile {
            max_output_tokens: Some(0),
            ..base.clone()
        },
        AnalysisProfile {
            image_size: ImageSizePolicy::MaximumLongEdge(0),
            ..base.clone()
        },
    ] {
        assert!(invalid.validate().is_err());
    }
    assert!(
        validate_analysis_profiles(&[base.clone(), AnalysisProfile { id: 2, ..base }]).is_err()
    );
}
