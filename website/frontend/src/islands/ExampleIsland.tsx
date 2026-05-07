import { createSignal } from "solid-js";

import type { ExampleIslandPropsJson } from "../gen/props/v1/example_pb";

// Demo island used to prove that SSR placeholders become interactive Solid UI,
// including when HTMX inserts a fresh placeholder after the initial page load.
export function ExampleIsland(props: ExampleIslandPropsJson) {
  const [clicks, setClicks] = createSignal(props.initialCount ?? 0);

  return (
    <section class="space-y-3">
      <div>
        <p class="text-xs font-semibold uppercase tracking-[0.22em] text-base-content/60">
          Solid island
        </p>
        <h2 class="text-xl font-semibold text-base-content">
          {props.label ?? "Solid island mounted."}
        </h2>
      </div>
      <button class="btn btn-secondary" type="button" onClick={() => setClicks((count) => count + 1)}>
        Local clicks: {clicks()}
      </button>
    </section>
  );
}
