use crate::Route;
use dioxus::prelude::*;

const NAVBAR_CSS: Asset = asset!("/assets/styling/navbar.css");

/// The Navbar component that will be rendered on all pages of our app since every page is under the layout.
///
///
/// This layout component wraps the UI of [Route::Home] and [Route::Blog] in a common navbar. The contents of the Home and Blog
/// routes will be rendered under the outlet inside this component
#[component]
pub fn Navbar() -> Element {
    let mut theme = use_context::<Signal<String>>();
    let theme_name = theme();
    let next_theme = if theme_name == "goodfox" {
        "badfox"
    } else {
        "goodfox"
    };

    rsx! {
        document::Link { rel: "stylesheet", href: NAVBAR_CSS }

        div {
            id: "navbar",
            class: "navbar bg-base-200 text-base-content px-4 shadow-sm",
            div {
                class: "flex-1 gap-2",
                Link {
                    class: "btn btn-ghost",
                    to: Route::Home {},
                    "Home"
                }
                Link {
                    class: "btn btn-ghost",
                    to: Route::Blog { id: 1 },
                    "Blog"
                }
            }
            button {
                class: "btn btn-primary btn-sm",
                onclick: move |_| theme.set(next_theme.to_string()),
                "Use {next_theme}"
            }
        }

        div {
            class: "mx-auto mt-6 w-full max-w-5xl px-4",
            div {
                class: "alert alert-info mb-6",
                span { "Current DaisyUI theme: {theme_name}" }
            }

            // The `Outlet` component is used to render the next component inside the layout. In this case, it will render either
            // the [`Home`] or [`Blog`] component depending on the current route.
            Outlet::<Route> {}
        }
    }
}
