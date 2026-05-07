use hypertext::{prelude::*, validation::Attribute};
use serde::Serialize;

trait SolidAttributes: GlobalAttributes {
    #[allow(non_upper_case_globals)]
    const solid_island: Attribute = Attribute;
    #[allow(non_upper_case_globals)]
    const solid_props: Attribute = Attribute;
}

impl<T: GlobalAttributes> SolidAttributes for T {}

pub trait IslandName {
    const NAME: &'static str;
}

pub fn host<I: IslandName>() -> impl Renderable {
    rsx! { <div solid-island=(I::NAME)></div> }
}

pub fn host_with_props<I: IslandName, P: Serialize>(props: &P) -> impl Renderable + use<I, P> {
    let props = serde_json::to_string(props).expect("island props must serialize to JSON");

    rsx! { <div solid-island=(I::NAME) solid-props=(props)></div> }
}

#[cfg(test)]
mod tests {
    use crate::http::client::{islands, props};

    use super::*;

    // Ensure the generated island type renders the mount contract expected by the browser runtime.
    #[test]
    fn host_renders_a_solid_host() {
        let html = host::<islands::ExampleIsland>().render();

        assert!(html.as_inner().contains(r#"solid-island="ExampleIsland""#));
    }

    #[test]
    fn host_with_props_renders_serialized_proto_props() {
        let html = host_with_props::<islands::ExampleIsland, _>(&props::ExampleIslandProps {
            label: "Example".to_owned(),
            initial_count: 3,
        })
        .render();

        assert!(html.as_inner().contains(r#"solid-island="ExampleIsland""#));
        assert!(html.as_inner().contains(
            r#"solid-props="{&quot;label&quot;:&quot;Example&quot;,&quot;initialCount&quot;:3}"#
        ));
    }
}
