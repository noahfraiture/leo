use backend::analysis::{Judgment, RunStage};

pub fn judgment_label(judgment: Judgment) -> &'static str {
    match judgment {
        Judgment::Met => "Met",
        Judgment::NotMet => "Not met",
        Judgment::CannotVerify => "Cannot verify",
    }
}

pub fn judgment_class(judgment: Judgment) -> &'static str {
    match judgment {
        Judgment::Met => "badge badge-success",
        Judgment::NotMet => "badge badge-error",
        Judgment::CannotVerify => "badge badge-warning",
    }
}

pub fn stage_label(stage: RunStage) -> &'static str {
    match stage {
        RunStage::Preparing => "Preparing evidence",
        RunStage::Reading => "Independent readers",
        RunStage::Reviewing => "Reviewing evidence",
        RunStage::Gathering => "Gathering more evidence",
        RunStage::FinalReview => "Final review",
        RunStage::Ready => "Ready for review",
        RunStage::Paused => "Processing paused",
    }
}

/// Session-relative time shown consistently in citations, playback bounds, and recording durations.
pub fn timeline(milliseconds: u64) -> String {
    let seconds = milliseconds / 1000;
    if seconds >= 3600 {
        format!(
            "{}:{:02}:{:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    } else {
        format!("{}:{:02}", seconds / 60, seconds % 60)
    }
}

/// Evidence timestamps retain milliseconds so nearby samples are distinguishable.
pub fn timestamp(milliseconds: u64) -> String {
    format!("{}.{:03}", timeline(milliseconds), milliseconds % 1000)
}

pub fn recorded_at(milliseconds: i64) -> String {
    chrono::DateTime::from_timestamp_millis(milliseconds)
        .map(|time| {
            time.with_timezone(&chrono::Local)
                .format("%d %b %Y, %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "Unknown recording date".into())
}
