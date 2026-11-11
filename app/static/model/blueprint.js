// The shipped blueprints (vault.md#project, blueprints.rs), from the page's
// data block (#blueprints, page.rs): what New project offers, and the block
// a project of one starts with.

import { $ } from '../core/dom.js';

export const BLUEPRINTS = JSON.parse($('blueprints')?.textContent || '[]');

// The block a new project of `blueprint` starts with: its name and what it
// takes from it (its checks, its labels), so a release changes no project.
export function newProjectBlock(blueprint) {
  const block = { blueprint: blueprint.name };
  if (blueprint.checks.length) block.checks = blueprint.checks;
  if (blueprint.labels.length) block.labels = blueprint.labels;
  return block;
}
