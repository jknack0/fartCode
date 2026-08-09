// fartCode-ticket-edit block detection + parse. The PM agent replies to a
// ticket-change request with one fenced ```fartCode-ticket-edit JSON block;
// the transcript renders it as an approval card and Apply patches the
// issue via issue_update. Parse failure renders raw — never throws.

const FENCE = /```fartCode-ticket-edit[^\S\n]*\n([\s\S]*?)```/g;

export interface TicketEdit {
  issueId: string;
  /** null = unchanged. body/acceptance are FULL replacements. */
  title: string | null;
  body: string | null;
  acceptance: string[] | null;
}

/** Raw JSON payloads of every fartCode-ticket-edit block in `text`. */
export function extractTicketEditBlocks(text: string): string[] {
  return [...text.matchAll(FENCE)].map((m) => m[1].trim());
}

/** `text` with all fartCode-ticket-edit blocks removed. */
export function stripTicketEditBlocks(text: string): string {
  return text.replace(FENCE, "").trim();
}

/** Strict parse; null on anything malformed. */
export function parseTicketEdit(raw: string): TicketEdit | null {
  let v: unknown;
  try {
    v = JSON.parse(raw);
  } catch {
    return null;
  }
  if (typeof v !== "object" || v === null) return null;
  const o = v as Record<string, unknown>;
  if (typeof o.issueId !== "string" || o.issueId.length === 0) return null;
  const str = (x: unknown) => (typeof x === "string" ? x : null);
  const acceptance = Array.isArray(o.acceptance)
    ? o.acceptance.every((a) => typeof a === "string")
      ? (o.acceptance as string[])
      : null
    : null;
  if (o.acceptance != null && acceptance === null) return null;
  const edit: TicketEdit = {
    issueId: o.issueId,
    title: str(o.title),
    body: str(o.body),
    acceptance,
  };
  if (edit.title === null && edit.body === null && edit.acceptance === null) return null;
  return edit;
}
