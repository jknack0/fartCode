// Thin typed wrappers over the ade Tauri commands + the event channel.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface ProjectDto {
  id: string;
  name: string;
  path: string;
  workspaceProvider: string;
  baseRef: string | null;
  repositoryWorkspaceId: string | null;
  createdAt: string | null;
  updatedAt: string | null;
}

export interface TaskDto {
  id: string;
  projectId: string;
  name: string;
  status: string;
  linkedIssue: unknown;
  archivedAt: string | null;
  isPinned: boolean;
  lastInteractedAt: string | null;
  statusChangedAt: string | null;
  workspaceId: string | null;
  createdBy: string;
  type: string;
}

export type AdeEvent =
  | { type: "project:added"; id: string; name: string; path: string }
  | { type: "project:deleted"; id: string }
  | { type: "task:created"; id: string; projectId: string; name: string }
  | { type: "task:deleted"; taskId: string }
  | { type: "task:status_changed"; taskId: string; status: string }
  | { type: "git:changed"; projectId: string; workspaceId: string }
  | { type: "files:changed"; workspaceId: string; paths: string[] }
  | { type: "comment:created"; id: string; taskId: string; filePath: string; lineNumber: number }
  | { type: "comment:resolved"; id: string }
  | { type: "issue:created"; id: string; projectId: string; title: string }
  | { type: "issue:updated"; id: string; projectId: string }
  | { type: "issue:deleted"; id: string; projectId: string };

export function listProjects(): Promise<ProjectDto[]> {
  return invoke("list_projects");
}
export function createProject(path: string): Promise<ProjectDto> {
  return invoke("create_project", { path });
}
export function deleteProject(id: string): Promise<void> {
  return invoke("delete_project", { id });
}
export function createTask(projectId: string, name: string): Promise<TaskDto> {
  return invoke("create_task", { projectId, name });
}

export function listTasks(projectId: string): Promise<TaskDto[]> {
  return invoke("list_tasks", { projectId });
}

/** Idempotently materializes the task's workspace (worktree + branch). */
export function provisionTask(taskId: string): Promise<void> {
  return invoke("provision_task", { taskId });
}
export function togglePin(id: string): Promise<TaskDto> {
  return invoke("toggle_pin", { id });
}

export interface DeleteTaskOptions {
  deleteWorktree?: boolean;
  deleteBranch?: boolean;
}

export function deleteTask(
  projectId: string,
  taskId: string,
  options: DeleteTaskOptions = {},
): Promise<void> {
  return invoke("delete_task", {
    projectId,
    taskId,
    deleteWorktree: options.deleteWorktree ?? null,
    deleteBranch: options.deleteBranch ?? null,
  });
}

/** Subscribe to backend events; returns an unsubscribe fn. */
export function onAdeEvent(cb: (event: AdeEvent) => void): Promise<() => void> {
  return listen<AdeEvent>("ade:event", (e) => cb(e.payload));
}

// -- Project settings (E1-05) ------------------------------------------------

export interface ScriptsDto {
  setup?: string | null;
  run?: string | null;
  teardown?: string | null;
}

export interface WorkspaceProviderDto {
  type: string;
  provisionCommand?: string | null;
  terminateCommand?: string | null;
}

/** defaultBranch is untagged: "main" | { name, remote } */
export type DefaultBranchDto = string | { name: string; remote: boolean };

export interface ProjectSettingsDto {
  worktreeDirectory?: string | null;
  defaultBranch?: DefaultBranchDto | null;
  baseRemote?: string | null;
  pushRemote?: string | null;
  githubAccountId?: string | null;
  tmux?: boolean | null;
  autoRunSetupScriptOnTaskCreation?: boolean | null;
  autoRunRunScriptOnTaskCreation?: boolean | null;
  taskStartupCommand?: string | null;
  workspaceProvider?: WorkspaceProviderDto | null;
  preservePatterns?: string[] | null;
  shellSetup?: string | null;
  scripts?: ScriptsDto | null;
}

export function getProjectSettings(projectId: string): Promise<ProjectSettingsDto> {
  return invoke("get_project_settings", { projectId });
}
export function updateProjectSettings(
  projectId: string,
  settings: ProjectSettingsDto,
): Promise<ProjectSettingsDto> {
  return invoke("update_project_settings", { projectId, settings });
}
export function shareWithTeam(projectId: string): Promise<boolean> {
  return invoke("share_with_team", { projectId });
}

// -- View state (E1-08) -------------------------------------------------------

export function getViewState(key: string): Promise<unknown> {
  return invoke("get_view_state", { key });
}
export function setViewState(key: string, value: unknown): Promise<void> {
  return invoke("set_view_state", { key, value });
}

// -- Search + resource monitor (E1-09) ---------------------------------------

export interface SearchResultDto {
  itemType: string;
  itemId: string;
  projectId: string | null;
  taskId: string | null;
  title: string;
}

export interface ResourceSampleDto {
  cpuPercent: number;
  memUsedMb: number;
  memTotalMb: number;
}

export function search(query: string, limit?: number): Promise<SearchResultDto[]> {
  return invoke("search", { query, limit });
}
export function resourceSample(): Promise<ResourceSampleDto> {
  return invoke("resource_sample");
}
export function getResourceMonitorEnabled(): Promise<boolean> {
  return invoke("get_resource_monitor_enabled");
}
export function setResourceMonitorEnabled(enabled: boolean): Promise<void> {
  return invoke("set_resource_monitor_enabled", { enabled });
}

// -- E3-07 provider accounts -------------------------------------------------

export interface ProviderAccountDto {
  id: string;
  providerId: string;
  accountId: string;
  label: string | null;
  isDefault: boolean;
  /** Server-computed mask of the keyring secret — never the secret. */
  maskedSecret: string;
  createdAt: number;
  updatedAt: number;
}

export function listProviderAccounts(
  providerId: string | null,
): Promise<ProviderAccountDto[]> {
  return invoke("list_provider_accounts", { providerId });
}

export function addProviderAccount(
  providerId: string,
  accountId: string,
  secret: string,
  label: string | null,
): Promise<ProviderAccountDto> {
  return invoke("add_provider_account", { providerId, accountId, secret, label });
}

export function removeProviderAccount(id: string): Promise<void> {
  return invoke("remove_provider_account", { id });
}

export function setDefaultProviderAccount(id: string): Promise<void> {
  return invoke("set_default_provider_account", { id });
}

export interface ProviderDto {
  id: string;
  name: string;
  description: string;
  websiteUrl: string | null;
  capabilities: string[];
  models: string[];
  defaultModel: string | null;
  binaries: string[];
  promptStrategy: string;
}

export function listProviders(): Promise<ProviderDto[]> {
  return invoke("list_providers");
}

// -- E2-12 interactive terminals ---------------------------------------------

export function terminalOpen(
  taskId: string,
  rows: number,
  cols: number,
): Promise<string> {
  return invoke("terminal_open", { taskId, rows, cols });
}

export function terminalWrite(terminalId: string, data: string): Promise<void> {
  return invoke("terminal_write", { terminalId, data });
}

export function terminalResize(
  terminalId: string,
  cols: number,
  rows: number,
): Promise<void> {
  return invoke("terminal_resize", { terminalId, cols, rows });
}

export function terminalClose(terminalId: string): Promise<void> {
  return invoke("terminal_close", { terminalId });
}

/** Count of the task's tmux sessions that survive but this app process does
 * not currently show (ADR-0028) — restore opens extras for each. */
export function terminalSurviving(taskId: string): Promise<number> {
  return invoke("terminal_surviving", { taskId });
}

/** A task terminal: `kind` is `shell` | `agent` | `lifecycle` (script
 * terminals stay listed after the script exits so their tab can reattach
 * the output tail — E1-06). */
export interface TaskTerminalDto {
  id: string;
  agent: string | null;
  kind: string;
  scriptType: string | null;
}

/** Lists the task's terminals (diff selection routing + restore). */
export function terminalListForTask(taskId: string): Promise<TaskTerminalDto[]> {
  return invoke("terminal_list_for_task", { taskId });
}

/** Runs the task's lifecycle script (`setup`/`run`/`teardown`) as a
 * terminal in the task worktree (env contract included); reattaches an
 * in-flight run of the same type. Returns the terminal id. */
export function terminalOpenLifecycle(
  taskId: string,
  scriptType: string,
  rows: number,
  cols: number,
): Promise<string> {
  return invoke("terminal_open_lifecycle", { taskId, scriptType, rows, cols });
}

/** Base64 scrollback tail replayed when reattaching to a live terminal
 * after a frontend reload; `null` for unknown/empty. */
export function terminalTail(terminalId: string): Promise<string | null> {
  return invoke("terminal_tail", { terminalId });
}

/** Subscribe to terminal output chunks (base64); returns an unsubscribe fn. */
export function onTerminalOutput(
  cb: (payload: { terminalId: string; data: string }) => void,
): Promise<() => void> {
  return listen<{ terminalId: string; data: string }>("terminal:output", (e) =>
    cb(e.payload),
  );
}

/** Subscribe to terminal exit notices; returns an unsubscribe fn. */
export function onTerminalExited(
  cb: (payload: { terminalId: string; exitCode: number | null }) => void,
): Promise<() => void> {
  return listen<{ terminalId: string; exitCode: number | null }>(
    "terminal:exited",
    (e) => cb(e.payload),
  );
}

export function terminalOpenAgent(
  taskId: string,
  agent: string,
  rows: number,
  cols: number,
): Promise<string> {
  return invoke("terminal_open_agent", { taskId, agent, rows, cols });
}

// -- E2-11-5 conversations + ACP ---------------------------------------------

/** One conversation row for the frontend. `runtime` is the provider
 * decision's output: "acp" runs structured chat, "tui" keeps the terminal
 * path — the frontend routes ⌘Enter on this field. */
export interface ConversationDto {
  id: string;
  projectId: string;
  taskId: string | null;
  /** "task" | "project" (E17-04: the PM chat is project-scoped). */
  scope: string;
  title: string;
  provider: string | null;
  type: "pty" | "acp";
  sessionId: string | null;
  runtime: "tui" | "acp";
}

/** The reduced transcript + live models (`acp_history` / `acp:transcript`
 * payload). Serialized camelCase from `ade_acp::LiveModels`. */
export interface LiveModels {
  sessionState: {
    lifecycle: "starting" | "ready" | "working" | "cancelling" | "closed";
    activeTurnId: string | null;
    pendingPermissions: PendingPermission[];
    lastStopReason: string | null;
    queuedPrompts: QueuedPromptDto[];
    isGenerating: boolean;
    canSubmit: boolean;
    canCancel: boolean;
  };
  committed: TranscriptTurn[];
  activeTurn: TranscriptTurn | null;
  config: SessionConfigState;
  usage: SessionUsage | null;
  title: string | null;
  agents: unknown[];
  plan: PlanState | null;
  draft: { text: string; rev: number } | null;
  queuedPrompts: QueuedPromptDto[];
  terminals: unknown[];
}

export interface QueuedPromptDto {
  id: string;
  text: string;
  hiddenContext: string | null;
}

/** The tool call a permission request gates (ACP ToolCallUpdate — the
 * fields the prompt card renders; the full update carries more). */
export interface PermissionToolCall {
  toolCallId?: string;
  title?: string;
  kind?: string;
  status?: string;
  [key: string]: unknown;
}

export interface PendingPermission {
  requestId: string;
  sessionId: string;
  toolCall: PermissionToolCall;
  options: { optionId: string; name: string; kind: string }[];
}

/** One reduced turn. Items are the serializer's untagged union — `kind`
 * discriminates ("message" | "thinking" | a tool-call kind | "tool-group");
 * shapes mirror `ade_acp::transcript::models` exactly. */
export interface TranscriptTurn {
  id: string;
  seq: number;
  initiator: "user" | "agent";
  items: TranscriptItem[];
  outcome?:
    | { kind: "done"; reason?: string }
    | { kind: "cancelled"; reason?: string }
    | { kind: "error"; reason?: string }
    | { kind: "interrupted"; reason?: string };
}

export type TranscriptItem =
  | MessageItem
  | ThinkingItem
  | ToolCallItem
  | ToolGroupItem;

/** A tool node in a nested tree (group children). */
export type ToolNode = ToolCallItem | ToolGroupItem;

export interface MessageItem {
  kind: "message";
  id: string;
  seq: number;
  role: "user" | "assistant";
  text: string;
}

export interface ThinkingItem {
  kind: "thinking";
  id: string;
  seq: number;
  segmentId: string;
  text: string;
  status: "thinking" | "done";
  startedAt: number;
  durationMs?: number;
}

export type ToolItemKind =
  | "execute-tool-call"
  | "read-tool-call"
  | "create-file-tool-call"
  | "modify-file-tool-call"
  | "delete-file-tool-call"
  | "search-tool-call"
  | "mcp-tool-call"
  | "web-fetch-tool-call"
  | "spawn-subagent-tool-call"
  | "create-plan-tool-call"
  | "unknown-tool-call";

export type ToolStatus = "running" | "done" | "error";

/** One tool call row. Kind-specific fields are absent unless the kind
 * carries them (flattened union, ADR-0029). */
export interface ToolCallItem {
  kind: ToolItemKind;
  id: string;
  seq: number;
  toolCallId: string;
  title: string;
  status: ToolStatus;
  inputSummary?: string;
  parentToolCallId?: string;
  children?: ToolNode[];
  // execute
  command?: string;
  outputText?: string;
  terminalId?: string;
  // read / create-file / modify-file / delete-file
  path?: string;
  content?: string;
  oldText?: string;
  newText?: string;
  // search
  query?: string;
  matchCount?: number;
  // mcp
  tool?: string;
  server?: string;
  // web fetch
  url?: string;
  pageTitle?: string;
  // subagent / unknown
  name?: string;
  agentId?: string;
  background?: boolean;
  // plan
  planId?: string;
  // unknown
  toolKind?: string;
}

/** Collapsed batch of sibling tool calls (e.g. read-batch). */
export interface ToolGroupItem {
  kind: "tool-group";
  id: string;
  seq: number;
  label: string;
  groupKind: string;
  status: ToolStatus;
  children: ToolNode[];
}

export interface SessionUsage {
  contextSize: number;
  contextUsed: number;
  cost: { amount: number; currency: string } | null;
}

/** Session-scoped plan slice (latest full snapshot per update). */
export interface PlanState {
  id: string;
  entries: PlanEntry[];
  updatedAt: number;
}

export interface PlanEntry {
  id: string;
  content: string;
  status: "pending" | "in_progress" | "completed";
  priority: "high" | "medium" | "low";
}

export interface SessionConfigState {
  modelOptions: SelectGroup | null;
  efforts: SelectGroup | null;
  modeOptions: SelectGroup | null;
  availableCommands: {
    name: string;
    description: string;
    source: string;
    inputHint?: string;
  }[];
}

export interface SelectGroup {
  configId: string;
  selected: string | null;
  available: { id: string; name: string; description?: string }[];
}

export function createConversation(
  projectId: string,
  taskId: string,
  provider: string | null,
  title?: string,
): Promise<ConversationDto> {
  return invoke("create_conversation", { projectId, taskId, provider, title });
}

export function listConversations(taskId: string): Promise<ConversationDto[]> {
  return invoke("list_conversations", { taskId });
}

export function acpStart(
  conversationId: string,
): Promise<{ sessionId: string; resumed: boolean }> {
  return invoke("acp_start", { conversationId });
}

export function acpSendPrompt(
  conversationId: string,
  text: string,
  hiddenContext?: string,
): Promise<{ queued: boolean }> {
  return invoke("acp_send_prompt", { conversationId, text, hiddenContext });
}

export function acpCancel(conversationId: string): Promise<void> {
  return invoke("acp_cancel", { conversationId });
}

export function acpResolvePermission(
  conversationId: string,
  requestId: string,
  optionId: string,
): Promise<void> {
  return invoke("acp_resolve_permission", { conversationId, requestId, optionId });
}

export function acpStop(conversationId: string): Promise<void> {
  return invoke("acp_stop", { conversationId });
}

export function acpHistory(conversationId: string): Promise<LiveModels | null> {
  return invoke("acp_history", { conversationId });
}

/** One routed raw `session/update` (debug/streaming channel). */
export interface AcpUpdatePayload {
  conversationId: string;
  sessionId: string;
  update: { sessionUpdate: string; [key: string]: unknown };
}

/** Subscribe to raw ACP updates; returns an unsubscribe fn. */
export function onAcpUpdate(
  cb: (payload: AcpUpdatePayload) => void,
): Promise<() => void> {
  return listen<AcpUpdatePayload>("acp:update", (e) => cb(e.payload));
}

/** Subscribe to transcript/live-model snapshots; returns an unsubscribe fn. */
export function onAcpTranscript(
  cb: (payload: { conversationId: string; models: LiveModels }) => void,
): Promise<() => void> {
  return listen<{ conversationId: string; models: LiveModels }>(
    "acp:transcript",
    (e) => cb(e.payload),
  );
}

/** Subscribe to permission requests; returns an unsubscribe fn. */
export function onAcpPermissionRequest(
  cb: (payload: {
    conversationId: string;
    requestId: string;
    pending: PendingPermission;
  }) => void,
): Promise<() => void> {
  return listen<{
    conversationId: string;
    requestId: string;
    pending: PendingPermission;
  }>("acp:permission_request", (e) => cb(e.payload));
}

// -- E4 Git & diff (shapes mirror ade-git status.rs / diff.rs exactly) --------

export interface GitChangeDto {
  path: string;
  origPath: string | null;
  status: "added" | "modified" | "deleted" | "renamed" | "conflicted";
  additions: number;
  deletions: number;
}

export interface GitStatusSnapshot {
  staged: GitChangeDto[];
  unstaged: GitChangeDto[];
  stagedAdditions: number;
  stagedDeletions: number;
  truncated: boolean;
}

export type DiffSide = "staged" | "unstaged";

export interface FileDiffDto {
  path: string;
  origPath: string | null;
  oldContent: string | null;
  newContent: string | null;
  oldSize: number;
  newSize: number;
  oldExists: boolean;
  newExists: boolean;
  binary: boolean;
  tooLarge: boolean;
}

export function gitStatus(workspaceId: string): Promise<GitStatusSnapshot> {
  return invoke("git_status", { workspaceId });
}

export function gitFileDiff(
  workspaceId: string,
  path: string,
  side: DiffSide,
  origPath?: string | null,
): Promise<FileDiffDto> {
  return invoke("git_file_diff", {
    workspaceId,
    path,
    side,
    origPath: origPath ?? null,
  });
}

export function gitStage(workspaceId: string, paths: string[]): Promise<void> {
  return invoke("git_stage", { workspaceId, paths });
}
export function gitStageAll(workspaceId: string): Promise<void> {
  return invoke("git_stage_all", { workspaceId });
}
export function gitUnstage(workspaceId: string, paths: string[]): Promise<void> {
  return invoke("git_unstage", { workspaceId, paths });
}
export function gitDiscard(workspaceId: string, paths: string[]): Promise<void> {
  return invoke("git_discard", { workspaceId, paths });
}

// -- E4-06 commit card --------------------------------------------------------

/** Repo state driving the commit card's disabled matrix + PR-open guard. */
export interface GitCommitStateDto {
  branch: string | null;
  remote: string | null;
  hasRemote: boolean;
  published: boolean;
  prOpen: boolean;
  canCreatePr: boolean;
  /** E4-08 footer state: upstream shorthand, ahead/behind, remotes. */
  upstream: string | null;
  ahead: number;
  behind: number;
  remotes: string[];
}
export interface GitCommitResultDto {
  hash: string;
}
export interface GitPushOutcomeDto {
  branch: string;
  remote: string;
  setUpstream: boolean;
  prUrl: string | null;
}

export function gitCommitState(workspaceId: string): Promise<GitCommitStateDto> {
  return invoke("git_commit_state", { workspaceId });
}
export function gitCommit(workspaceId: string, message: string): Promise<GitCommitResultDto> {
  return invoke("git_commit", { workspaceId, message });
}
export interface GitCreatePrOutcomeDto {
  url: string;
  pushed: boolean;
}

export function gitPush(workspaceId: string): Promise<GitPushOutcomeDto> {
  return invoke("git_push", { workspaceId });
}
export function gitCreatePr(workspaceId: string): Promise<GitCreatePrOutcomeDto> {
  return invoke("git_create_pr", { workspaceId });
}

// -- E4-08 footer git actions ---------------------------------------------------

export interface GitPublishOutcomeDto {
  branch: string;
  remote: string;
}

export function gitFetch(workspaceId: string): Promise<void> {
  return invoke("git_fetch", { workspaceId });
}
export function gitPull(workspaceId: string): Promise<void> {
  return invoke("git_pull", { workspaceId });
}
export function gitPublish(workspaceId: string): Promise<GitPublishOutcomeDto> {
  return invoke("git_publish", { workspaceId });
}
export function gitAddRemote(workspaceId: string, name: string, url: string): Promise<void> {
  return invoke("git_add_remote", { workspaceId, name, url });
}

/** Left-nav project pull: `git pull --ff-only` at the project root. */
export function projectGitPull(projectId: string): Promise<void> {
  return invoke("project_git_pull", { projectId });
}

/** The project's browsable GitHub URL, or null when the base remote isn't
 * GitHub (project view hides the icon). */
export function projectGithubUrl(projectId: string): Promise<string | null> {
  return invoke("project_github_url", { projectId });
}

/** Writes the worktree side of a diff back to disk (E4-05 ⌘S). */
export function writeWorkspaceFile(
  workspaceId: string,
  path: string,
  content: string,
): Promise<void> {
  return invoke("write_workspace_file", { workspaceId, path, content });
}

// -- E4-10 line comments (ARCHITECTURE.md §14) ----------------------------------

export interface LineCommentDto {
  id: string;
  taskId: string;
  filePath: string;
  lineNumber: number;
  lineEnd: number | null;
  sourceSide: "before" | "after";
  lineContent: string | null;
  content: string;
  resolved: boolean;
  resolvedAt: string | null;
  createdBy: string;
  linkedTaskId: string | null;
  createdAt: string | null;
  updatedAt: string | null;
}

export function addLineComment(args: {
  taskId: string;
  filePath: string;
  lineNumber: number;
  lineEnd: number | null;
  sourceSide: "before" | "after";
  lineContent: string | null;
  content: string;
}): Promise<LineCommentDto> {
  return invoke("add_line_comment", { request: args });
}

export function listLineComments(taskId: string, filePath?: string): Promise<LineCommentDto[]> {
  return invoke("list_line_comments", { taskId, filePath: filePath ?? null });
}

export function resolveLineComment(commentId: string): Promise<LineCommentDto> {
  return invoke("resolve_line_comment", { commentId });
}

export function deleteLineComment(commentId: string): Promise<void> {
  return invoke("delete_line_comment", { commentId });
}

/** §14 "Create Task": provisioned task + the constructed initial prompt. */
export interface CreateTaskFromCommentResult {
  task: TaskDto;
  prompt: string;
}

export function createTaskFromComment(args: {
  projectId: string;
  name: string;
  commentId: string;
  selectedCode: string;
  enclosingFunction: string | null;
}): Promise<CreateTaskFromCommentResult> {
  return invoke("create_task_from_comment", args);
}

// -- E17 project board + PM chat (ARCHITECTURE.md §13, ADR-0032) ----------------

export type Lane = "backlog" | "ready" | "in_progress" | "in_review" | "done";

export interface BlockerRefDto {
  id: string;
  title: string;
  lane: Lane;
}

/** One board card. `blocked` is derived server-side (any blocker lane ≠
 * done); `blockers` feeds the badge hover list. */
export interface IssueDto {
  id: string;
  projectId: string;
  title: string;
  body: string | null;
  acceptance: string[];
  lane: Lane;
  position: number;
  provider: string | null;
  model: string | null;
  prdPath: string | null;
  prdSection: string | null;
  linkedTaskId: string | null;
  blocked: boolean;
  blockers: BlockerRefDto[];
  createdAt: string | null;
  updatedAt: string | null;
}

export function issueList(projectId: string): Promise<IssueDto[]> {
  return invoke("issue_list", { projectId });
}

export function issueCreate(args: {
  projectId: string;
  title: string;
  body?: string | null;
  acceptance?: string[];
  lane?: Lane;
  provider?: string | null;
  model?: string | null;
  prdPath?: string | null;
  prdSection?: string | null;
}): Promise<IssueDto> {
  return invoke("issue_create", { request: args });
}

/** Field patch: omit a key to leave it; explicit null clears a nullable
 * field (title/acceptance are non-nullable). */
export function issueUpdate(
  issueId: string,
  patch: {
    title?: string;
    body?: string | null;
    acceptance?: string[];
    provider?: string | null;
    model?: string | null;
    prdPath?: string | null;
    prdSection?: string | null;
  },
): Promise<IssueDto> {
  return invoke("issue_update", { issueId, patch });
}

/** Lane move (board drag). position omitted = append to lane end. */
export function issueMove(
  issueId: string,
  lane: Lane,
  position?: number,
): Promise<IssueDto> {
  return invoke("issue_move", { issueId, lane, position: position ?? null });
}

export function issueDelete(issueId: string): Promise<void> {
  return invoke("issue_delete", { issueId });
}

/** `issueId` becomes blocked by `blockedById`; cycle/cross-project
 * rejections surface as thrown errors. */
export function issueLink(issueId: string, blockedById: string): Promise<IssueDto> {
  return invoke("issue_link", { issueId, blockedById });
}

export function issueUnlink(issueId: string, blockedById: string): Promise<IssueDto> {
  return invoke("issue_unlink", { issueId, blockedById });
}

/** Project-scoped (PM chat) conversations (E17-04). */
export function listProjectConversations(projectId: string): Promise<ConversationDto[]> {
  return invoke("list_project_conversations", { projectId });
}

/** The project's one persistent PM conversation, created on first call.
 * `provider` must be ACP-capable (throws otherwise). */
export function getOrCreateProjectConversation(
  projectId: string,
  provider: string,
): Promise<ConversationDto> {
  return invoke("get_or_create_project_conversation", { projectId, provider });
}

// -- E17-04 PM proposal blocks (ADR-0032) ---------------------------------------

export interface ProposalPrdDto {
  path: string;
  title: string | null;
}

export interface ProposalIssueDto {
  title: string;
  body: string | null;
  acceptance: string[];
  blockedBy: string[];
  provider: string | null;
  model: string | null;
}

export interface ProposalDto {
  prd: ProposalPrdDto | null;
  issues: ProposalIssueDto[];
}

/** Validates a raw ade-proposal block payload. Rejects (throws) on
 * malformed input — the caller renders the block as plain text instead. */
export function issueParseProposal(text: string): Promise<ProposalDto> {
  return invoke("issue_parse_proposal", { text });
}

/** Applies an approved proposal (issues + blocked-by edges, all-or-nothing).
 * Returns the created issues. */
export function issueApplyProposal(
  projectId: string,
  proposal: ProposalDto,
): Promise<IssueDto[]> {
  return invoke("issue_apply_proposal", { projectId, proposal });
}
