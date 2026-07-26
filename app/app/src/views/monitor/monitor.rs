use crate::components::Camera;
use dioxus::prelude::*;

#[component]
pub fn Monitor() -> Element {
    rsx! {
        Camera { }
    }
}
