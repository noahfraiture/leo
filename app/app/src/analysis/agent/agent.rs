use super::error::Result;
use crate::analysis::video::FrameSet;

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
}

struct Prompt {}

impl Prompt {
    fn builder() -> PromptBuilder {
        todo!()
    }
}

struct PromptBuilder {}

impl PromptBuilder {
    fn add_frame_set(mut self, frame: FrameSet) -> Self {
        todo!()
    }

    fn overlap_prompt(mut self, prompt: Prompt) -> Self {
        todo!()
    }

    fn build(self) -> Prompt {
        todo!()
    }
}
