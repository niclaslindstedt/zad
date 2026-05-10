// Typed wrapper around the JSON the extractor writes. The shape mirrors
// `website/scripts/extract-source-data.mjs`; if those drift apart the
// build fails at compile time instead of at runtime.

import raw from "../generated/sourceData.json";

export interface CargoMetadata {
  version: string;
  license: string;
  repository: string;
  rustVersion: string;
  edition: string;
  cliDescription: string;
  libDescription: string;
}

export interface CommandNode {
  name: string;
  path: string[];
  description?: string;
  about?: string;
  args?: unknown[];
  subcommands?: CommandNode[];
}

export interface CommandTree {
  commands: CommandNode[];
  [key: string]: unknown;
}

export interface ChangelogEntry {
  version: string | null;
  date: string | null;
  body: string;
}

export interface SourceData {
  name: string;
  generatedAt: string;
  cargo: CargoMetadata;
  services: string[];
  commands: CommandTree;
  quickStart: string;
  librarySnippet: string;
  changelog: ChangelogEntry | null;
  docs: string[];
  manpages: string[];
  examples: string[];
}

export const sourceData = raw as unknown as SourceData;

export const cargo = sourceData.cargo;
export const version = cargo.version;
export const repository = cargo.repository;
export const services = sourceData.services;
export const commandTree = sourceData.commands;
export const quickStart = sourceData.quickStart;
export const librarySnippet = sourceData.librarySnippet;
export const changelog = sourceData.changelog;
export const docsList = sourceData.docs;
export const manpagesList = sourceData.manpages;
export const examplesList = sourceData.examples;
export const generatedAt = sourceData.generatedAt;
