use super::error::Result;
use crate::analysis::video::FrameSet;

use bon::{bon, builder};
use rig_core::completion::CompletionModel;

struct Agent<M>
where
    M: CompletionModel,
{
    model: M,
}

impl<M> Agent<M>
where
    M: CompletionModel,
{
    fn new(model: M) -> Self {
        Self { model }
    }

    fn analyze(&self, prompt: Prompt) -> Result<Response> {
        todo!()
    }
}

struct Response {
    sequence_summary: String,
}

const INSTRUCTIONS: &'static str = r#"
You are an agent comparing an instructions correct video sequence to students video sequence.

Bellow find a description of the correct video sequence:
"""
{checklist}
"""

Find below the sequence summary until know with timestamp. There might be small overlap with the frames you receive:
"""
{previous_summary}
"""

Find attached frames representing a sample of the video sequence.
There can be frame from multiple camera at the same time.

// list of frames with the image name and the timestamp at which the frame was taken.
{frames}
"#;

struct Prompt<'a> {
    frame_set: FrameSet,
    checklist: &'a str,
    previous_summary: String,
}

#[bon]
impl<'a> Prompt<'a> {
    #[builder]
    fn new(frame_set: FrameSet, checklist: &'a str, response: Response) -> Self {
        Self {
            frame_set,
            checklist,
            previous_summary: response.sequence_summary,
        }
    }

    fn serialize(&self) -> String {
        format!(INSTRUCTIONS, self.checklist)
    }
}
