//! Thin, typed bridge to the Rust core (Tauri commands).
//! `invoke` failures from Tauri arrive as objects shaped like EngineError when
//! the Rust side rejects with `EngineError`; normalize anything else.

import { invoke } from "@tauri-apps/api/core";
import type { EngineError, OpenResult, CommitResult, PageMeta, Metadata } from "./types";
import { isEngineError } from "./types";

async function cmd<T>(name: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(name, args);
  } catch (e) {
    if (isEngineError(e)) {
      throw e as EngineError;
    }
    const err: EngineError = { code: "open_failed", message: String(e), detail: e };
    throw err;
  }
}

export interface OpenPayload extends OpenResult {}

export async function cmdOpen(path: string): Promise<OpenPayload> {
  return cmd<OpenPayload>("open_document", { path });
}

export async function cmdPageOps(kind: string, data: unknown): Promise<PageMeta[]> {
  return cmd<PageMeta[]>("page_op", { kind, data });
}

export async function cmdSetMetadata(meta: Metadata): Promise<PageMeta[]> {
  return cmd<PageMeta[]>("set_metadata", { meta });
}

export async function cmdCommit(target: string, overwrite: boolean): Promise<CommitResult> {
  return cmd<CommitResult>("commit_export", { target, overwrite });
}

export async function cmdSourceHash(): Promise<{ source_hash: string }> {
  return cmd<{ source_hash: string }>("source_hash");
}