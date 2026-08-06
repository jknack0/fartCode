// ade-proposal block detection (E17-04, #58; ADR-0032). The PM agent emits
// fenced ```ade-proposal JSON blocks in its replies; the transcript renders
// each as an approval card (or plain text when the backend parse rejects
// it). Detection here is intentionally dumb — validation is the backend's
// issue_parse_proposal, this only extracts candidate payloads.

const FENCE = /```ade-proposal[^\S\n]*\n([\s\S]*?)```/g;

/** Raw JSON payloads of every ade-proposal block in `text`. */
export function extractProposalBlocks(text: string): string[] {
  const out: string[] = [];
  for (const match of text.matchAll(FENCE)) {
    out.push(match[1].trim());
  }
  return out;
}

/** `text` with all ade-proposal blocks removed (the transcript renders the
 * card in their place; stray prose around the block still shows). */
export function stripProposalBlocks(text: string): string {
  return text.replace(FENCE, "").trim();
}
