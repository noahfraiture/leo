use dioxus::prelude::*;

#[component]
pub fn Camera() -> Element {
    rsx! {
        div {
            class: "card card-border bg-base-100 w-full",

            figure {
                class: "relative aspect-video bg-base-200",

                div {
                    class: "absolute inset-4 flex flex-col justify-between",

                    div {
                        class: "flex justify-between",

                        div {
                            class: "badge badge-outline",
                            span { class: "status status-success" }
                            "LIVE"
                        }

                        span { "14:42:18" }
                    }

                    div {
                        class: "badge badge-primary badge-outline",
                        "Selected"
                    }
                }
            }

            div {
                class: "card-body",

                div {
                    class: "flex items-center justify-between",

                    h2 {
                        class: "card-title",
                        span { class: "status status-success" }
                        "Workshop"
                    }

                    div {
                        class: "card-actions",
                        button {
                            class: "btn btn-ghost btn-circle btn-sm",
                            aria_label: "Camera options",
                            "..."
                        }
                    }
                }

                p { "CAM 04 - Selected camera" }
            }
        }
    }
}
