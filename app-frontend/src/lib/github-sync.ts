// GitHub issue import (E17): pull every open issue of the project's
// checkout onto the board, deduped by URL. Runs ONCE, when a project is
// added (store/sidebar.ts) — it used to autorun on every board mount,
// which re-imported on each board↔task bounce and manufactured step
// launches that yanked the selection into whichever project synced.
// Failures stay quiet: the board works fine offline/local.
import { issueImportGithub, issueList, projectGithubIssues } from "./tauri";

export async function syncGithubIssues(projectId: string): Promise<void> {
  const [ghIssues, board] = await Promise.all([
    projectGithubIssues(projectId),
    issueList(projectId),
  ]);
  const imported = new Set(
    board.filter((i) => i.externalRef).map((i) => i.externalRef),
  );
  for (const g of ghIssues.filter((x) => !imported.has(x.url))) {
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
}
