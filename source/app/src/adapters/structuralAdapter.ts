//! Structural adapter — OpenDocumentAdapter / PageTreeAdapter / MetadataAdapter
//! / CommitAdapter (STAGE0_ENGINE_INTERFACES §2, §4, §5, §6) implemented over
//! the Rust core (Tauri IPC). `extract` is model-only until CommitAdapter runs,
//! and every export path lands in the Rust atomic `commit_export`.

import {
  Document,
  PageTreeAdapter,
  MetadataAdapter,
  CommitAdapter,
  OpenDocumentAdapter,
  OpenResult,
  PageState,
} from "../types";
import {
  cmdCommit,
  cmdMerge,
  cmdOpen,
  cmdPageOps,
  cmdSetMetadata,
  cmdSourceHash,
} from "../ipc";

function beforePages(doc: Document): number[] {
  return doc.pages.map((p) => p.page_no);
}

/** Project an engine `OpenResult` into the frontend document model. */
export function documentFromOpen(open: OpenResult, base?: Document): Document {
  return {
    sourceHash: open.source_hash,
    sourceSize: open.source_size,
    path: open.path,
    metadata: open.metadata,
    pages: open.pages.map((p) => ({ ...p })),
    encrypted: open.encrypted,
    passwordRequired: open.password_required,
    warnings: open.warnings,
    bytesB64: open.bytes_b64,
    bookmarks: base?.path === open.path ? base.bookmarks : [],
    history: base?.path === open.path ? base.history : [],
    redo: base?.path === open.path ? base.redo : [],
  };
}

export const openDocumentAdapter: OpenDocumentAdapter = {
  async open(path) {
    return documentFromOpen(await cmdOpen(path));
  },
  async verifyIntegrity(doc) {
    const { source_hash } = await cmdSourceHash();
    return {
      sha256: source_hash,
      size: doc.sourceSize,
      pages: doc.pages.length,
      warnings: doc.warnings,
    };
  },
};

export const pageTreeAdapter: PageTreeAdapter = {
  async reorder(doc, newOrder) {
    const before = beforePages(doc);
    await cmdPageOps("reorder", { new_order: newOrder });
    return { kind: "reorder", before, data: { new_order: newOrder } };
  },
  async rotate(doc, pageNumbers, cw) {
    const before = beforePages(doc);
    await cmdPageOps("rotate", { pages: pageNumbers, cw });
    return { kind: "rotate", before, data: { pages: pageNumbers, cw } };
  },
  async delete(doc, pageNumbers) {
    const before = beforePages(doc);
    await cmdPageOps("delete", { pages: pageNumbers });
    return { kind: "delete", before, data: { pages: pageNumbers } };
  },
  async insertBlank(doc, afterPage, widthPt, heightPt) {
    const before = beforePages(doc);
    await cmdPageOps("insert_blank", {
      after: afterPage,
      width_pt: widthPt,
      height_pt: heightPt,
    });
    return { kind: "insert_blank", before, data: { after: afterPage, width_pt: widthPt, height_pt: heightPt } };
  },
  async extract(_doc, pageNumbers) {
    const pages: PageState[] = await cmdPageOps("extract", { pages: pageNumbers });
    return { pages, outSourceHash: "" };
  },
  async editText(doc, params) {
    const before = beforePages(doc);
    await cmdPageOps("edit_text", {
      page: params.page,
      x: params.x,
      y: params.y,
      width: params.width,
      height: params.height,
      font_size: params.font_size,
      new_text: params.new_text,
    });
    return { kind: "edit_text", before, data: params };
  },
  async merge(doc, path) {
    const before = beforePages(doc);
    await cmdMerge(path);
    return { kind: "merge", before, data: { path } };
  },
  getPage(doc, pageNumber) {
    return doc.pages.find((p) => p.page_no === pageNumber);
  },
};

export const metadataAdapter: MetadataAdapter = {
  get(doc) {
    return doc.metadata;
  },
  async set(doc, updates) {
    const before = beforePages(doc);
    await cmdSetMetadata(updates);
    return { kind: "set_metadata", before, data: updates };
  },
};

export const commitAdapter: CommitAdapter = {
  /** Refuses an export onto the opened source path (data-safety rule 5). */
  async validate(doc, target) {
    const norm = (p: string) => {
      try {
        return new URL(`file://${p}`).pathname;
      } catch {
        return p;
      }
    };
    if (norm(target) === norm(doc.path) && doc.path === target) {
      return { ok: false, reason: "attempted to save over the opened source file" };
    }
    if (doc.passwordRequired) {
      return { ok: false, reason: "document is password-protected; unlock before exporting" };
    }
    return { ok: true };
  },
  async export(doc, target, overwrite) {
    const check = await this.validate(doc, target);
    if (!check.ok) {
      const err: Error & { code?: string } = new Error(check.reason);
      err.code = "permission_denied";
      throw err;
    }
    return cmdCommit(target, overwrite);
  },
};