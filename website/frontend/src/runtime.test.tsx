import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { Component } from "solid-js";

import { cleanupIslands, initializeIslandRuntime, mountIslands } from "./runtime";
import type { IslandRegistry } from "./islands";

const TestIsland: Component<{ message?: string }> = (props) => (
  <div>{props.message ?? "Mounted test island"}</div>
);

const registry: IslandRegistry = {
  TestIsland,
};

describe("island runtime", () => {
  beforeAll(() => {
    initializeIslandRuntime(document);
  });

  beforeEach(() => {
    document.body.innerHTML = "";
    vi.restoreAllMocks();
  });

  it("mounts a known island during the initial scan", () => {
    document.body.innerHTML = '<div solid-island="TestIsland"></div>';

    mountIslands(document, registry);

    expect(document.querySelector("[solid-island]")?.textContent).toContain(
      "Mounted test island",
    );
    expect(
      document.querySelector("[solid-island]")?.getAttribute("solid-mounted"),
    ).toBe("true");
  });

  it("passes parsed solid-props into the mounted island", () => {
    document.body.innerHTML =
      '<div solid-island="TestIsland" solid-props=\'{"message":"Props arrived"}\'></div>';

    mountIslands(document, registry);

    expect(document.querySelector("[solid-island]")?.textContent).toContain(
      "Props arrived",
    );
  });

  it("mounts islands inside a swapped subtree without scanning the whole document", () => {
    document.body.innerHTML = [
      '<div id="untouched"><div solid-island="TestIsland"></div></div>',
      '<div id="swapped"></div>',
    ].join("");

    const swappedRoot = document.getElementById("swapped");
    const untouchedRoot = document.getElementById("untouched");

    mountIslands(untouchedRoot!, registry);

    swappedRoot!.innerHTML = '<div solid-island="TestIsland"></div>';
    mountIslands(swappedRoot!, registry);

    expect(swappedRoot?.textContent).toContain("Mounted test island");
    expect(
      untouchedRoot?.querySelector("[solid-island]")?.getAttribute("solid-mounted"),
    ).toBe("true");
  });

  it("skips nodes that were already mounted", () => {
    document.body.innerHTML = '<div solid-island="TestIsland"></div>';

    mountIslands(document, registry);

    const island = document.querySelector("[solid-island]");
    const firstRender = island?.innerHTML;

    mountIslands(document, registry);

    expect(island?.innerHTML).toBe(firstRender);
  });

  it("disposes mounted roots and descendants during cleanup", () => {
    document.body.innerHTML = [
      '<div id="wrapper">',
      '  <div solid-island="TestIsland"></div>',
      '  <div><div solid-island="TestIsland"></div></div>',
      "</div>",
    ].join("");

    const wrapper = document.getElementById("wrapper");

    mountIslands(wrapper!, registry);
    cleanupIslands(wrapper!);

    const mountedAttrs = Array.from(
      wrapper!.querySelectorAll("[solid-island]"),
    ).map((element) => element.getAttribute("solid-mounted"));

    expect(mountedAttrs).toEqual([null, null]);
  });

  it("logs and skips unknown islands", () => {
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
    document.body.innerHTML = '<div solid-island="MissingIsland"></div>';

    mountIslands(document, registry);

    expect(consoleError).toHaveBeenCalledWith(
      'Unknown Solid island "MissingIsland".',
    );
    expect(document.querySelector("[solid-island]")?.innerHTML).toBe("");
  });

  it("logs and skips islands whose props cannot be parsed", () => {
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
    document.body.innerHTML = '<div solid-island="TestIsland" solid-props="{oops"></div>';

    mountIslands(document, registry);

    expect(consoleError).toHaveBeenCalled();
    expect(document.querySelector("[solid-island]")?.innerHTML).toBe("");
  });

  it("cleans and remounts only the normal HTMX swap target", () => {
    document.body.innerHTML = [
      '<section id="untouched"><div solid-island="ExampleIsland"></div></section>',
      '<section id="swapped"><div solid-island="ExampleIsland"></div></section>',
    ].join("");

    mountIslands(document);

    const untouchedHost = document.querySelector("#untouched [solid-island]");
    const swappedRoot = document.getElementById("swapped")!;
    const oldSwappedHost = document.querySelector("#swapped [solid-island]")!;

    document.body.dispatchEvent(
      new CustomEvent("htmx:beforeSwap", {
        detail: { target: swappedRoot },
      }),
    );

    expect(oldSwappedHost.getAttribute("solid-mounted")).toBeNull();
    expect(untouchedHost?.getAttribute("solid-mounted")).toBe("true");

    swappedRoot.innerHTML = '<div solid-island="ExampleIsland"></div>';

    document.body.dispatchEvent(
      new CustomEvent("htmx:afterSwap", {
        detail: { target: swappedRoot },
      }),
    );

    expect(swappedRoot.querySelector("[solid-island]")?.textContent).toContain(
      "Solid island mounted.",
    );
    expect(untouchedHost?.textContent).toContain("Solid island mounted.");
  });

  it("cleans and remounts every OOB target selected by id, class, and attribute selectors", () => {
    document.body.innerHTML = [
      '<section id="toast"><div solid-island="ExampleIsland"></div></section>',
      '<section class="status-region"><div solid-island="ExampleIsland"></div></section>',
      '<section class="status-region"><div solid-island="ExampleIsland"></div></section>',
      '<section data-banner="global"><div solid-island="ExampleIsland"></div></section>',
      '<section id="untouched"><div solid-island="ExampleIsland"></div></section>',
    ].join("");

    mountIslands(document);

    const idTarget = document.querySelector("#toast")!;
    const classTargets = Array.from(document.querySelectorAll(".status-region"));
    const attributeTarget = document.querySelector('[data-banner="global"]')!;
    const untouchedHost = document.querySelector("#untouched [solid-island]");

    const oobTargets = [
      idTarget,
      ...classTargets,
      attributeTarget,
    ];

    const oldHosts = oobTargets.map(
      (target) => target.querySelector("[solid-island]")!,
    );

    for (const target of oobTargets) {
      document.body.dispatchEvent(
        new CustomEvent("htmx:oobBeforeSwap", {
          detail: { target },
        }),
      );
    }

    for (const host of oldHosts) {
      expect(host.getAttribute("solid-mounted")).toBeNull();
    }
    expect(untouchedHost?.getAttribute("solid-mounted")).toBe("true");

    for (const [index, target] of oobTargets.entries()) {
      target.innerHTML = `<div solid-island="ExampleIsland" data-oob-target="${index}"></div>`;
      document.body.dispatchEvent(
        new CustomEvent("htmx:oobAfterSwap", {
          detail: { target },
        }),
      );
    }

    for (const target of oobTargets) {
      expect(target.querySelector("[solid-island]")?.textContent).toContain(
        "Solid island mounted.",
      );
      expect(
        target.querySelector("[solid-island]")?.getAttribute("solid-mounted"),
      ).toBe("true");
    }

    expect(untouchedHost?.getAttribute("solid-mounted")).toBe("true");
  });
});
