// GitHub issue import (E17): pull every open issue of the project's
// checkout onto the board, deduped by URL. Runs ONCE, when a project is
// added (store/sidebar.ts) — it used to autorun on every board mount,
// which re-imported on each board↔task bounce and manufactured step
// launches that yanked the selection into whichever project synced.
// #120: the auto-import is gated by the project's `autoImport` setting and
// both import + auto-pull outcomes report through the board's quiet status
// line (store/ui.ts `projectNotice`) instead of `console.warn`.
import { issueImportGithub, issueList, projectGithubIssues } from "./tauri";
import { useUi } from "../store/ui";

export async function syncGithubIssues(projectId: string): Promise<number> {
  const [ghIssues, board] = await Promise.all([
    projectGithubIssues(projectId),
    issueList(projectId),
  ]);
  const imported = new Set(
    board.filter((i) => i.externalRef).map((i) => i.externalRef),
  );
  const fresh = ghIssues.filter((x) => !imported.has(x.url));
  for (const g of fresh) {
    await issueImportGithub({
      projectId,
      number: g.number,
      title: g.title,
      url: g.url,
      body: g.body,
      labels: g.labels,
      assignees: g.assignees,
      milestone: g.milestone,
    });
  }
  return fresh.length;
}

/** Import + report through the board's quiet status line (#120). Returns
 * the number newly imported so callers can skip the notice when needed. */
export async function importGithubIssues(projectId: string): Promise<number> {
  const n = await syncGithubIssues(projectId);
  useUi.getState().setProjectNotice(
    n > 0 ? `imported ${n} GitHub issue${n === 1 ? "" : "s"}` : "no new GitHub issues",
  );
  return n;
}
