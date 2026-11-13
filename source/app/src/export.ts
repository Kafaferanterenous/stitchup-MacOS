//! Exports: Text, CSV, RTF, DOCX, ODT (from the pdf.js text layer) and page
//! images (PNG/JPEG). Office formats are written as STORE-only ZIP packages
//! built here — no third-party ZIP or Office library (same approach as the
//! Windows build).

import type { RenderSession } from "./adapters/renderAdapter";

// ------------------------------------------------------------------ text

/** Lines of text per page, in reading order as pdf.js reports them. */
export async function pageLines(session: RenderSession): Promise<string[][]> {
  const out: string[][] = [];
  for (let p = 1; p <= session.pdf.numPages; p++) {
    const tc = await (await session.pdf.getPage(p)).getTextContent();
    const lines: string[] = [];
    let cur = "";
    for (const it of tc.items as Array<{ str?: string; hasEOL?: boolean }>) {
      if (typeof it.str !== "string") continue;
      cur += it.str;
      if (it.hasEOL) {
        lines.push(cur.replace(/\s+$/u, ""));
        cur = "";
      }
    }
    if (cur.trim()) lines.push(cur.replace(/\s+$/u, ""));
    out.push(lines);
  }
  return out;
}

const enc = new TextEncoder();
const BOM = new Uint8Array([0xef, 0xbb, 0xbf]);

function withBom(s: string): Uint8Array {
  const b = enc.encode(s);
  const out = new Uint8Array(BOM.length + b.length);
  out.set(BOM);
  out.set(b, BOM.length);
  return out;
}

/** UTF-8 (BOM) text; pages separated by a form feed. */
export function toText(pages: string[][]): Uint8Array {
  return withBom(pages.map((l) => l.join("\n")).join("\n\f\n") + "\n");
}

/** UTF-8 (BOM) CSV, one row per page (same columns as the Windows build). */
export async function toCsv(session: RenderSession, pages: string[][]): Promise<Uint8Array> {
  const rows = ["Page,Width (pt),Height (pt),Text chars,Annotations"];
  for (let p = 1; p <= session.pdf.numPages; p++) {
    const page = await session.pdf.getPage(p);
    const vp = page.getViewport({ scale: 1 });
    const annots = (await page.getAnnotations()).length;
    const chars = pages[p - 1].join("").length;
    rows.push(`${p},${vp.width.toFixed(1)},${vp.height.toFixed(1)},${chars},${annots}`);
  }
  return withBom(rows.join("\r\n") + "\r\n");
}

function rtfEscape(s: string): string {
  let out = "";
  for (const ch of s) {
    const c = ch.codePointAt(0)!;
    if (ch === "\\" || ch === "{" || ch === "}") out += "\\" + ch;
    else if (c < 128) out += ch;
    else if (c <= 0xffff) out += `\\u${c > 32767 ? c - 65536 : c}?`;
    else {
      // Astral: UTF-16 surrogate pair.
      const hi = Math.floor((c - 0x10000) / 0x400) + 0xd800;
      const lo = ((c - 0x10000) % 0x400) + 0xdc00;
      out += `\\u${hi - 65536}?\\u${lo - 65536}?`;
    }
  }
  return out;
}

export function toRtf(pages: string[][]): Uint8Array {
  const body = pages
    .map((lines) => lines.map((l) => rtfEscape(l) + "\\par").join("\n"))
    .join("\n\\page\n");
  return enc.encode(`{\\rtf1\\ansi\\ansicpg1252\\deff0{\\fonttbl{\\f0 Helvetica;}}\\f0\\fs22\n${body}\n}`);
}

const xml = (s: string) =>
  s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]!).replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f]/g, "");

export function toDocx(pages: string[][]): Uint8Array {
  const paras: string[] = [];
  pages.forEach((lines, i) => {
    if (i > 0) paras.push('<w:p><w:r><w:br w:type="page"/></w:r></w:p>');
    for (const l of lines) paras.push(`<w:p><w:r><w:t xml:space="preserve">${xml(l)}</w:t></w:r></w:p>`);
  });
  const files: [string, string][] = [
    [
      "[Content_Types].xml",
      `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>`,
    ],
    [
      "_rels/.rels",
      `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>`,
    ],
    [
      "word/document.xml",
      `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>${paras.join("")}</w:body></w:document>`,
    ],
  ];
  return zip(files.map(([n, c]) => [n, enc.encode(c)]));
}

export function toOdt(pages: string[][]): Uint8Array {
  const paras: string[] = [];
  pages.forEach((lines, i) => {
    lines.forEach((l, j) =>
      paras.push(`<text:p${i > 0 && j === 0 ? ' text:style-name="PB"' : ""}>${xml(l)}</text:p>`),
    );
    if (lines.length === 0) paras.push(`<text:p${i > 0 ? ' text:style-name="PB"' : ""}/>`);
  });
  const ns =
    'xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:version="1.2"';
  const content = `<?xml version="1.0" encoding="UTF-8"?><office:document-content ${ns}><office:automatic-styles><style:style style:name="PB" style:family="paragraph"><style:paragraph-properties fo:break-before="page"/></style:style></office:automatic-styles><office:body><office:text>${paras.join("")}</office:text></office:body></office:document-content>`;
  const manifest = `<?xml version="1.0" encoding="UTF-8"?><manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.2"><manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.text"/><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/></manifest:manifest>`;
  // `mimetype` must be the first entry and stored uncompressed (ODF §3.3).
  return zip([
    ["mimetype", enc.encode("application/vnd.oasis.opendocument.text")],
    ["META-INF/manifest.xml", enc.encode(manifest)],
    ["content.xml", enc.encode(content)],
  ]);
}

// ------------------------------------------------------------------- zip

const CRC_TABLE = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();

export function crc32(data: Uint8Array): number {
  let c = 0xffffffff;
  for (const b of data) c = CRC_TABLE[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

/** STORE-only ZIP (no compression) — valid for OOXML and ODF packages. */
export function zip(files: [string, Uint8Array][]): Uint8Array {
  const parts: Uint8Array[] = [];
  const central: Uint8Array[] = [];
  let offset = 0;
  const u16 = (v: number) => [v & 0xff, (v >>> 8) & 0xff];
  const u32 = (v: number) => [v & 0xff, (v >>> 8) & 0xff, (v >>> 16) & 0xff, (v >>> 24) & 0xff];
  // 1980-01-01 00:00 in DOS date/time.
  const time = 0;
  const date = (0 << 9) | (1 << 5) | 1;
  for (const [name, data] of files) {
    const nameBytes = enc.encode(name);
    const crc = crc32(data);
    const common = [...u16(20), ...u16(0x0800), ...u16(0), ...u16(time), ...u16(date), ...u32(crc), ...u32(data.length), ...u32(data.length), ...u16(nameBytes.length)];
    const local = new Uint8Array([...u32(0x04034b50), ...common, ...u16(0)]);
    parts.push(local, nameBytes, data);
    central.push(
      new Uint8Array([...u32(0x02014b50), ...u16(20), ...common, ...u16(0), ...u16(0), ...u16(0), ...u16(0), ...u32(0), ...u32(offset)]),
      nameBytes,
    );
    offset += local.length + nameBytes.length + data.length;
  }
  const cdSize = central.reduce((a, b) => a + b.length, 0);
  const end = new Uint8Array([...u32(0x06054b50), ...u16(0), ...u16(0), ...u16(files.length), ...u16(files.length), ...u32(cdSize), ...u32(offset), ...u16(0)]);
  const all = [...parts, ...central, end];
  const out = new Uint8Array(all.reduce((a, b) => a + b.length, 0));
  let p = 0;
  for (const a of all) {
    out.set(a, p);
    p += a.length;
  }
  return out;
}

// ---------------------------------------------------------------- images

/** Render one page (its displayed rotation) to PNG/JPEG bytes at `dpi`. */
export async function pageImage(
  session: RenderSession,
  pageNo: number,
  rotation: number,
  dpi: number,
  type: "image/png" | "image/jpeg",
): Promise<Uint8Array> {
  const page = await session.pdf.getPage(pageNo);
  const vp = page.getViewport({ scale: dpi / 72, rotation });
  const canvas = document.createElement("canvas");
  canvas.width = Math.ceil(vp.width);
  canvas.height = Math.ceil(vp.height);
  const ctx = canvas.getContext("2d")!;
  // JPEG has no alpha: paint paper white first.
  ctx.fillStyle = "#fff";
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  await page.render({ canvasContext: ctx, viewport: vp }).promise;
  const blob: Blob = await new Promise((res, rej) => canvas.toBlob((b) => (b ? res(b) : rej(new Error("encode failed"))), type, 0.92));
  return new Uint8Array(await blob.arrayBuffer());
}

export function toBase64(bytes: Uint8Array): string {
  let s = "";
  const CH = 0x8000;
  for (let i = 0; i < bytes.length; i += CH) s += String.fromCharCode(...bytes.subarray(i, i + CH));
  return btoa(s);
}

/** Bounding box of non-white pixels of a page (PDF user space), for Auto-Crop. */
export async function inkBox(session: RenderSession, pageNo: number): Promise<[number, number, number, number] | null> {
  const page = await session.pdf.getPage(pageNo);
  const vp = page.getViewport({ scale: 2, rotation: 0 });
  const canvas = document.createElement("canvas");
  canvas.width = Math.ceil(vp.width);
  canvas.height = Math.ceil(vp.height);
  const ctx = canvas.getContext("2d", { willReadFrequently: true })!;
  ctx.fillStyle = "#fff";
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  await page.render({ canvasContext: ctx, viewport: vp }).promise;
  const { data, width, height } = ctx.getImageData(0, 0, canvas.width, canvas.height);
  let x0 = width, y0 = height, x1 = -1, y1 = -1;
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      const i = (y * width + x) * 4;
      if (data[i] < 245 || data[i + 1] < 245 || data[i + 2] < 245) {
        if (x < x0) x0 = x;
        if (x > x1) x1 = x;
        if (y < y0) y0 = y;
        if (y > y1) y1 = y;
      }
    }
  }
  if (x1 < 0) return null;
  const [ax, ay] = vp.convertToPdfPoint(x0, y1 + 1);
  const [bx, by] = vp.convertToPdfPoint(x1 + 1, y0);
  const m = 6; // keep a small margin
  return [Math.min(ax, bx) - m, Math.min(ay, by) - m, Math.max(ax, bx) + m, Math.max(ay, by) + m];
}
