import type { Component } from "solid-js";
import { render } from "solid-js/web";

import { islandRegistry, type IslandRegistry } from "./islands";

type Dispose = () => void;
type HtmxEventDetail = {
  elt?: Element;
  target?: Element;
};

const mountedIslands = new WeakMap<Element, Dispose>();
const initializedDocuments = new WeakSet<Document>();

// Accept only node types that can be scanned for island hosts.
function isSearchRoot(node: unknown): node is ParentNode {
  return node instanceof Document || node instanceof DocumentFragment || node instanceof Element;
}

// Collect island hosts from a root, including the root itself when it is a host.
function collectIslandRoots(root: ParentNode): Element[] {
  const nodes: Element[] = [];

  if (root instanceof Element && root.hasAttribute("solid-island")) {
    nodes.push(root);
  }

  if ("querySelectorAll" in root) {
    nodes.push(...Array.from(root.querySelectorAll("[solid-island]")));
  }

  return nodes;
}

// Resolve a DOM subtree from HTMX event detail fields in a defined priority order.
function resolveRoot(event: Event, detailKeys: Array<keyof HtmxEventDetail>): ParentNode | null {
  if (event instanceof CustomEvent) {
    const detail = event.detail as HtmxEventDetail | undefined;

    if (detail) {
      for (const key of detailKeys) {
        const candidate = detail[key];

        if (candidate && isSearchRoot(candidate)) {
          return candidate;
        }
      }
    }
  }

  return null;
}

// Map a server-declared island name onto a registered Solid component.
function resolveComponent(element: Element, registry: IslandRegistry): Component | undefined {
  const islandName = element.getAttribute("solid-island");

  if (!islandName) {
    return undefined;
  }

  const component = registry[islandName];

  if (!component) {
    console.error(`Unknown Solid island "${islandName}".`);
  }

  return component;
}

// Parse serialized island props from the server-rendered host.
function resolveProps(element: Element): Record<string, unknown> | null {
  const rawProps = element.getAttribute("solid-props");

  if (!rawProps) {
    return {};
  }

  try {
    const parsed = JSON.parse(rawProps) as unknown;

    if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
      return parsed as Record<string, unknown>;
    }

    console.error("Solid island props must decode to an object.");
    return null;
  } catch (error) {
    console.error("Failed to parse Solid island props.", error);
    return null;
  }
}

// Mount every unmounted island host found in the provided subtree.
export function mountIslands(root: ParentNode = document, registry: IslandRegistry = islandRegistry): void {
  for (const element of collectIslandRoots(root)) {
    // HTMX may swap the same subtree multiple times, so mounting must be idempotent.
    if (element.getAttribute("solid-mounted") === "true" || mountedIslands.has(element)) {
      continue;
    }

    const component = resolveComponent(element, registry);

    if (!component) {
      continue;
    }

    const Component = component;
    const props = resolveProps(element);

    if (!props) {
      continue;
    }

    const dispose = render(() => <Component {...props} />, element as HTMLElement);

    mountedIslands.set(element, dispose);
    element.setAttribute("solid-mounted", "true");
  }
}

// Dispose a single mounted island and clear its bookkeeping.
function cleanupElement(element: Element): void {
  const dispose = mountedIslands.get(element);

  if (!dispose) {
    return;
  }

  dispose();
  mountedIslands.delete(element);
  element.removeAttribute("solid-mounted");
}

// Dispose every mounted island found in the provided subtree.
export function cleanupIslands(root: ParentNode): void {
  for (const element of collectIslandRoots(root)) {
    cleanupElement(element);
  }
}

// Wire the runtime into document load and HTMX swap/remove lifecycle events.
export function initializeIslandRuntime(doc: Document = document, registry: IslandRegistry = islandRegistry): void {
  if (initializedDocuments.has(doc)) {
    return;
  }

  const start = () => {
    initializedDocuments.add(doc);
    mountIslands(doc, registry);

    // Clean up before HTMX removes or replaces DOM so Solid can dispose effects cleanly.
    doc.body.addEventListener("htmx:beforeSwap", (event) => {
      const root = resolveRoot(event, ["target", "elt"]);

      if (root) {
        cleanupIslands(root);
      }
    });

    doc.body.addEventListener("htmx:beforeCleanupElement", (event) => {
      const root = resolveRoot(event, ["elt", "target"]);

      if (root) {
        cleanupIslands(root);
      }
    });

    // Mount only inside the swapped subtree instead of re-scanning the whole document.
    doc.body.addEventListener("htmx:afterSwap", (event) => {
      const root = resolveRoot(event, ["target", "elt"]);

      if (root) {
        mountIslands(root, registry);
      }
    });

    // OOB swaps target DOM elsewhere in the page, so they need their own scoped cleanup.
    doc.body.addEventListener("htmx:oobBeforeSwap", (event) => {
      const root = resolveRoot(event, ["target", "elt"]);

      if (root) {
        cleanupIslands(root);
      }
    });

    // OOB insertions mount only in the explicit OOB target selected by HTMX.
    doc.body.addEventListener("htmx:oobAfterSwap", (event) => {
      const root = resolveRoot(event, ["target", "elt"]);

      if (root) {
        mountIslands(root, registry);
      }
    });
  };

  if (doc.readyState === "loading") {
    doc.addEventListener("DOMContentLoaded", start, { once: true });
    return;
  }

  start();
}
