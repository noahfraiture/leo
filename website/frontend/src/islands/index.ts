import type { Component } from "solid-js";

// Islands are mounted from JSON at runtime, so the untyped boundary lives here
// instead of leaking into each island component.
export type IslandComponent = Component<any>;
export type IslandRegistry = Record<string, IslandComponent>;

type IslandModule = Record<string, unknown>;

const islandModules = import.meta.glob<IslandModule>(
  ["./*.tsx", "!./*.test.tsx"],
  {
    eager: true,
  },
);

// Derive the registry from island filenames so frontend and backend both use the same naming convention.
function islandNameFromPath(path: string): string {
  return path
    .split("/")
    .pop()!
    .replace(/\.tsx$/, "");
}

// Require each island module to export a component with the same name as its file.
function resolveIslandComponent(
  path: string,
  module: IslandModule,
): IslandComponent {
  const islandName = islandNameFromPath(path);
  const component = module[islandName];

  if (typeof component !== "function") {
    throw new Error(
      `Island module "${path}" must export a component named "${islandName}".`,
    );
  }

  return component as IslandComponent;
}

export const islandRegistry: IslandRegistry = Object.freeze(
  Object.fromEntries(
    Object.entries(islandModules).map(([path, module]) => [
      islandNameFromPath(path),
      resolveIslandComponent(path, module),
    ]),
  ),
);
