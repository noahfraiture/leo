#[derive(Default)]
pub struct Camera {
    pub(crate) status: Status,
}

impl Camera {
    pub fn new() -> Self {
        Self {
            status: Status::Ready,
        }
    }

    pub fn pan(&mut self) {}
}

#[derive(Clone, Default, PartialEq)]
pub(crate) enum Status {
    Running,
    #[default]
    Ready,
}
