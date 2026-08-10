// Thin typed wrappers over the fartCode Tauri commands + the event channel.
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

export type FartcodeEvent =
  | { type: "project:added"; id: string; name: string; path: string }
  | { type: "project:deleted"; id: string }
  | { type: "task:created"; id: string; projectId: string; name: string }
  | { type: "task:deleted"; taskId: string }
  | { type: "task:status_changed"; taskId: string; status: string }
  | { type: "task:archived"; taskId: string }
  | { type: "task:restored"; taskId: string }
  | { type: "git:changed"; projectId: string; workspaceId: string }
  | { type: "files:changed"; workspaceId: string; paths: string[] }
  | { type: "comment:created"; id: string; taskId: string; filePath: string; lineNumber: number }
  | { type: "comment:resolved"; id: string }
  | { type: "pr:updated"; workspaceId: string; prUrl: string }
  | { type: "issue:created"; id: string; projectId: string; title: string }
  | { type: "issue:updated"; id: string; projectId: string }
  | { type: "issue:deleted"; id: string; projectId: string }
  // E18-04/05 step engine (fartcode-app/src/app.rs `event_to_value`).
  // `step:launch` is a DIRECTIVE — open/focus the task's agent session;
  // the rest are notifications. Note a command-path launch emits the event
  // AND returns the same payload in `EnterOutcomeDto.launch`, so a
  // consumer that acts on both must dedupe (BoardView does).
  | {
      type: "step:launch";
      issueId: string;
      projectId: string;
      columnId: string;
      taskId: string;
      prompt: string;
      provider: string;
      model: string | null;
      effort: string | null;
      reattached: boolean;
    }
  | {
      type: "step:queued";
      issueId: string;
      projectId: string;
      columnId: string;
      provider: string;
      model: string | null;
      effort: string | null;
    }
  | { type: "step:queue_cleared"; issueId: string; projectId: string; columnId: string }
  /** The step settled and its column HOLDS — the card's step-done dot
   * (v3 system rules). Derived state: nothing is stored, so it lives only
   * for this session. */
  | { type: "step:settled"; issueId: string; projectId: string; columnId: string; taskId: string }
  | { type: "setting:changed"; key: string };

export function listProjects(): Promise<ProjectDto[]> {
  return invoke("list_projects");
}
export function createProject(path: string): Promise<ProjectDto> {
  return invoke("create_project", { path });
}
export function deleteProject(id: string): Promise<void> {
  return invoke("delete_project", { id });
}
export type WorkspaceKind = "new-worktree" | "project-root";
export interface CreateTaskOptions {
  workspace?: WorkspaceKind;
  /** Existing branch to check out in the worktree (default: new branch off the base ref). */
  branch?: string;
}
export function createTask(
  projectId: string,
  name: string,
  opts: CreateTaskOptions = {},
): Promise<TaskDto> {
  return invoke("create_task", {
    projectId,
    name,
    workspace: opts.workspace ?? null,
    branch: opts.branch ?? null,
  });
}
export interface BranchRef {
  name: string;
  upstream: string | null;
  is_remote: boolean;
  object: string;
}
export function listProjectBranches(projectId: string): Promise<BranchRef[]> {
  return invoke("list_project_branches", { projectId });
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

/** 7a "a archive instead": the card leaves the board; worktree + branch
 * survive. Emits `task:archived`. */
export function archiveTask(id: string): Promise<void> {
  return invoke("task_archive", { taskId: id });
}

/** ⌘K restore: clears `archivedAt`, the task returns. Emits `task:restored`. */
export function restoreTask(id: string): Promise<void> {
  return invoke("task_restore", { taskId: id });
}

/** Subscribe to backend events; returns an unsubscribe fn. */
export function onFartcodeEvent(cb: (event: FartcodeEvent) => void): Promise<() => void> {
  return listen<FartcodeEvent>("fartcode:event", (e) => cb(e.payload));
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
  /** Feature-dossier consent (E19-01, ADR-0038 item 3). THREE states:
   * `true` write, `false` don't, `null`/absent = never asked — which the
   * backend fails closed on, so the feature stays inert until the
   * first-dispatch consent card (#74) answers it. */
  featureDossiers?: boolean | null;
  /** App-managed bookkeeping (E19-02): the feature-log scaffold version
   * last written into this project. Declared here ONLY so a full-replace
   * write round-trips it — `update_project_settings` overwrites the whole
   * row, so a DTO that omitted this key would clear the app's memory of
   * the scaffold and resurrect files the user deleted. Never a UI field. */
  featureLogSeededVersion?: number | null;
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

/** 7c `⇧⌘s share local values` (E1-02): moves local shareable values into
 * the repo `.fartCode.json` and clears them from the DB. */
export function projectSettingsShare(projectId: string): Promise<void> {
  return invoke("project_settings_share", { projectId });
}

/** Which source won each SHAREABLE setting key (`preservePatterns` /
 * `shellSetup` / `scripts`): "shared" = the `.fartCode.json` value wins
 * (7c green tag), "local" = a DB override beats it, "default" = neither
 * defines it. */
export function projectSettingsProvenance(
  projectId: string,
): Promise<Record<string, "default" | "shared" | "local">> {
  return invoke("project_settings_provenance", { projectId });
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
  /** The card a `feature` hit opens (handoff v3 §8h), resolved backend-side
   * from the item id. Null on every other item type. It arrives WITH the
   * hit so ↵ never has to wait on a second round trip — a feature row is
   * routable the moment it renders. */
  issueId: string | null;
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

export interface AuthMethodDto {
  id: string;
  name: string;
  description: string;
  /** "cli-login" (OAuth subscription) | "api-key" */
  kind: string;
}

export interface AuthStatusDto {
  providerId: string;
  authenticated: boolean;
  account: string | null;
  /** "oauth" | "apiKey" | "none" | "unknown" */
  method: string;
}

export interface ProviderAccountDto {
  id: string;
  providerId: string;
  accountId: string;
  label: string | null;
  isDefault: boolean;
  /** Server-computed mask of the keyring secret — never the secret. */
  maskedSecret: string;
  /** Auth method id ("claude-login" = OAuth sub, "anthropic-api-key", null = legacy). */
  authMethod: string | null;
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
  authMethod: string | null,
): Promise<ProviderAccountDto> {
  return invoke("add_provider_account", { providerId, accountId, secret, label, authMethod });
}

export function removeProviderAccount(id: string): Promise<void> {
  return invoke("remove_provider_account", { id });
}

export function setDefaultProviderAccount(id: string): Promise<void> {
  return invoke("set_default_provider_account", { id });
}

/** Live CLI login (OAuth) state for a provider, e.g. `claude auth status`. */
export function providerAuthStatus(providerId: string): Promise<AuthStatusDto> {
  return invoke("provider_auth_status", { providerId });
}

/** Opens the provider's interactive login flow (`claude auth login`) in a terminal. */
export function providerAuthLogin(
  providerId: string,
  taskId: string | null,
  rows: number,
  cols: number,
): Promise<string> {
  return invoke("provider_auth_login", { providerId, taskId, rows, cols });
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
  authMethods: AuthMethodDto[];
}

export function listProviders(): Promise<ProviderDto[]> {
  return invoke("list_providers");
}

/** ProjectSettings "Default agent · model" row: writes the app-wide
 * `defaultAgent` setting (validated against the provider registry).
 * Emits `setting:changed` with key `defaultAgent` — refetch on it. */
export function setDefaultAgent(providerId: string): Promise<void> {
  return invoke("set_default_agent", { providerId });
}

// -- 7d Agents on this machine (E3-02 host dependencies) ----------------------

/** One agent CLI row. `latest` is set only when cheaply known (the network
 * update check is a Phase-0 stub — currently always null, so hide "update
 * available" affordances until it lands). `acp` mirrors the provider
 * registry capability; `isDefault` = the `defaultAgent` app setting. */
export interface HostDependencyDto {
  providerId: string;
  name: string;
  version: string | null;
  path: string | null;
  installed: boolean;
  latest: string | null;
  acp: boolean;
  isDefault: boolean;
}

/** Registry tail counts (7d `+ 31 more in the registry · 22 acp`). */
export interface DependencyRegistrySummaryDto {
  total: number;
  acp: number;
}

/** Every registered agent CLI with detection state. `refresh` forces a
 * re-detect (PATH scan + version probes); default serves the 300s cache. */
export function hostDependencyList(refresh = false): Promise<HostDependencyDto[]> {
  return invoke("host_dependency_list", { refresh });
}

/** Installs the provider's CLI via its registry plan; resolves with the
 * re-detected row when the installer finishes. No progress events — the
 * install runner reports no percentage (see backend module docs). */
export function hostDependencyInstall(id: string): Promise<HostDependencyDto> {
  return invoke("host_dependency_install", { providerId: id });
}

/** Updates the provider's CLI; resolves with the re-detected row. */
export function hostDependencyUpdate(id: string): Promise<HostDependencyDto> {
  return invoke("host_dependency_update", { providerId: id });
}

export function hostDependencyRegistrySummary(): Promise<DependencyRegistrySummaryDto> {
  return invoke("host_dependency_registry_summary");
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
 * the output tail — E1-06). `running` is false only for retained finished
 * lifecycle entries; `exitCode` is set for those (7b `setup ✗ exit 1`),
 * null while running or when the OS reported none. */
export interface TaskTerminalDto {
  id: string;
  agent: string | null;
  kind: string;
  scriptType: string | null;
  running: boolean;
  exitCode: number | null;
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
 * payload). Serialized camelCase from `fartcode_acp::LiveModels`. */
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
 * shapes mirror `fartcode_acp::transcript::models` exactly. */
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

// -- E4 Git & diff (shapes mirror fartcode-git status.rs / diff.rs exactly) --------

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

// -- E4-07/E4-09 PR section (shapes mirror fartcode-core/src/github) ----------

export type PrStatus = "open" | "closed" | "merged";

export interface PrUserDto {
  login: string;
  avatarUrl: string | null;
  url: string | null;
}

export interface PrFileDto {
  filename: string;
  status: string;
  additions: number;
  deletions: number;
  patchElided: boolean;
}

export interface PrCommitDto {
  sha: string;
  subject: string;
  author: PrUserDto | null;
  authorName: string | null;
  date: string | null;
  url: string | null;
}

export interface PrCheckDto {
  id: string;
  name: string;
  status: string;
  conclusion: string | null;
  url: string | null;
  startedAt: string | null;
  completedAt: string | null;
  appName: string | null;
}

export interface PrCommentDto {
  id: string;
  kind: "issue" | "review";
  body: string;
  url: string | null;
  author: PrUserDto | null;
  path: string | null;
  line: number | null;
  createdAt: string | null;
  updatedAt: string | null;
}

/** One denormalized PR — the whole Pull Requests tab. */
export interface PrDto {
  number: number;
  title: string;
  url: string;
  status: PrStatus;
  draft: boolean;
  author: PrUserDto | null;
  baseRef: string;
  headRef: string;
  headOid: string;
  additions: number;
  deletions: number;
  changedFiles: number;
  commitCount: number;
  mergeableState: string | null;
  reviewDecision: string | null;
  createdAt: string | null;
  updatedAt: string | null;
  files: PrFileDto[];
  commits: PrCommitDto[];
  checks: PrCheckDto[];
  comments: PrCommentDto[];
}

/** Pull Requests tab payload: empty-state discriminators + the PR. */
export interface PrSectionDto {
  tokenPresent: boolean;
  repository: string | null;
  branch: string | null;
  pr: PrDto | null;
}

/** Cached read + background sync (tab open). */
export function prSectionGet(workspaceId: string): Promise<PrSectionDto> {
  return invoke("pr_section_get", { workspaceId });
}

/** Awaits one sync pass, then returns the cache (explicit refresh). */
export function prSectionSync(workspaceId: string): Promise<PrSectionDto> {
  return invoke("pr_section_sync", { workspaceId });
}

export interface GithubTokenStatusDto {
  connected: boolean;
  mask: string | null;
}

export function githubTokenStatus(): Promise<GithubTokenStatusDto> {
  return invoke("github_token_status");
}
export function githubTokenImport(): Promise<GithubTokenStatusDto> {
  return invoke("github_token_import");
}
export function githubTokenSet(token: string): Promise<GithubTokenStatusDto> {
  return invoke("github_token_set", { token });
}
export function githubTokenClear(): Promise<void> {
  return invoke("github_token_clear");
}

/** Left-nav project pull: `git pull --ff-only` at the project root. */
export function projectGitPull(projectId: string): Promise<void> {
  return invoke("project_git_pull", { projectId });
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

/** E4-11 agent-tool surface: validated against the task's workspace and
 * attributed to the provider. */
export function agentAddLineComment(args: {
  taskId: string;
  filePath: string;
  lineStart: number;
  lineEnd: number | null;
  sourceSide: "before" | "after";
  content: string;
  provider: string;
}): Promise<LineCommentDto> {
  return invoke("agent_add_line_comment", { request: args });
}

/** Parses `createdBy` ("user" | "agent:<provider>") into an author. */
export function commentAuthor(createdBy: string): { kind: "user" | "agent"; provider: string | null } {
  if (createdBy.startsWith("agent:")) {
    return { kind: "agent", provider: createdBy.slice("agent:".length) };
  }
  return { kind: "user", provider: null };
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
  /** The blocker's effective column (mirror first, seeded lane as the
   * fallback) — `lane` cannot name a non-seeded column, so this is the
   * only thing that can label the row with the column the card is
   * actually in. null when its lane maps to no column at all. */
  columnId: string | null;
  /** True when the blocker's column carries counts_as_done (E18-03,
   * ADR-0037 item 6) — key "is it finished?" off this, not the lane. */
  countsAsDone: boolean;
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
  /** Repo-relative path of the feature dossier (ADR-0038 item 1), e.g.
   * `docs/features/oauth-login.md`. Set when the dossier is born with the
   * worktree at the card's first agent-step entry; null on cards that were
   * never dispatched, projects that declined dossiers, or a failed write.
   * Read-only from the frontend — the dossier lifecycle owns it. */
  dossierPath: string | null;
  linkedTaskId: string | null;
  /** Source URL when imported from an external tracker (GitHub #import). */
  externalRef: string | null;
  /** Mirror pointer to the card's board column (E18-04): maintained by
   * the enter primitive (lane moves route through it); lane stays
   * authoritative until E18-07. null on never-moved cards. */
  columnId: string | null;
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

/** Result of a board dispatch (E17-03): the linked task + prompt packet
 * for the frontend to launch the agent with. `reattached` = the card's
 * task is already live — focus it, never spawn a second. */
export interface DispatchOutcomeDto {
  task: TaskDto;
  issue: IssueDto;
  prompt: string;
  provider: string;
  reattached: boolean;
}

/** Drag-into-In-Progress: creates the linked task (worktree + prompt) or
 * reattaches to the live one. */
export function issueDispatch(issueId: string): Promise<DispatchOutcomeDto> {
  return invoke("issue_dispatch", { issueId });
}

// -- E19 feature dossiers (ADR-0038; handoff v3 §8f + §8h) ----------------------

/** One `## Timeline` breadcrumb, already folded by the Rust parser: a step
 * launch and its settle arrive as ONE `launched → settled · <dur>` line. */
export interface DossierTimelineEntryDto {
  /** The stamp as written in the file — rendered when `at` is null. */
  stamp: string;
  /** RFC3339 UTC, or null when the stamp is not one we wrote. Explicitly
   * zoned: a bare "2026-08-09 12:34" parses as LOCAL time in the webview. */
  at: string | null;
  text: string;
  /** A launch with no settle: append `running · <elapsed>` off `at`. */
  running: boolean;
}

/** One agent-written `## <Column> — <date>` section. */
export interface DossierSectionDto {
  heading: string;
  body: string;
}

export interface DossierDto {
  /** Repo-relative path — what the file is called on the branch. */
  path: string;
  /** Absolute path of the copy that was read (worktree, or the project
   * checkout once the feature landed). */
  hostPath: string;
  timeline: DossierTimelineEntryDto[];
  /** Empty is ordinary: the agent skipped the append (§8f — render the
   * timeline and no inset section, never a nag). */
  sections: DossierSectionDto[];
}

/** The card's dossier, or null when it has none — declined consent, a
 * pre-E19 card, or a file that left with an unmerged branch. Null means
 * render NO dossier group at all, not an empty state. */
export function dossierRead(issueId: string): Promise<DossierDto | null> {
  return invoke("dossier_read", { issueId });
}

/** What a ⌘K `feature` hit needs beyond its indexed section heading and the
 * `issueId` the search hit already carries (§8h): the feature's own title
 * and the tracker ref the `#id` derives from.
 *
 * No `landed`: the tag needs an ancestry answer the app cannot give yet —
 * see `FeatureRowDto` in fartcode-app/src/commands/dossiers.rs. */
export interface FeatureRowDto {
  /** Echoes the `search_index` item id asked about. */
  itemId: string;
  issueId: string;
  /** The card's title — the "feature title" half of the row title. */
  title: string;
  externalRef: string | null;
}

export function dossierFeatureRows(itemIds: string[]): Promise<FeatureRowDto[]> {
  return invoke("dossier_feature_rows", { itemIds });
}

// -- E18 configurable pipeline columns (ADR-0037) -------------------------------
// Spike behind the seeded default: `lane` stays authoritative on IssueDto;
// columns are data the board does not render from yet.

export type ColumnKind = "shelf" | "agent_step" | "human_gate";
/** Step trigger: `run` fires on drop; `queue` confirms first. */
export type ColumnOnEnter = "run" | "queue";
/** Settle behavior: `hold` waits for a human drag; `advance` auto-moves. */
export type ColumnOnSettle = "hold" | "advance";

/** One board column (ADR-0037): kind + flags + agent-step config. */
export interface BoardColumnDto {
  id: string;
  projectId: string;
  name: string;
  position: number;
  kind: ColumnKind;
  countsAsDone: boolean;
  isLanding: boolean;
  onEnter: ColumnOnEnter;
  onSettle: ColumnOnSettle;
  /** Explicit advance target (same-project column id); null = the next
   * column by position. Seeded Quick targets Done. */
  advanceTo: string | null;
  /** null on an agent step = the built-in dispatch packet. */
  stepPrompt: string | null;
  stepProvider: string | null;
  stepModel: string | null;
  stepEffort: string | null;
  /** Tool allowlist for the step's agent session. null = unrestricted;
   * a list allows ONLY the listed tools (so [] allows none — a corrupt
   * stored allowlist also reads as [], failing closed). */
  stepTools: string[] | null;
  /** The legacy lane a seeded column mirrors ("backlog" | "ready" |
   * "in_progress" | "in_review" | "done"); null on Quick and user-created
   * columns. Read-only. */
  seedLane: Lane | null;
  createdAt: string | null;
  updatedAt: string | null;
}

/** Columns for a project in board order (position). */
export function columnList(projectId: string): Promise<BoardColumnDto[]> {
  return invoke("column_list", { projectId });
}

/** Creates a column appended to the end of the board. `isLanding: true`
 * moves the landing flag onto the new column (never duplicates it).
 * `advanceTo` must reference a same-project column. */
export function columnCreate(args: {
  projectId: string;
  name: string;
  kind: ColumnKind;
  countsAsDone?: boolean;
  isLanding?: boolean;
  onEnter?: ColumnOnEnter;
  onSettle?: ColumnOnSettle;
  advanceTo?: string | null;
  stepPrompt?: string | null;
  stepProvider?: string | null;
  stepModel?: string | null;
  stepEffort?: string | null;
  /** Omit/null = unrestricted; a list (even empty) = allowlist. */
  stepTools?: string[] | null;
}): Promise<BoardColumnDto> {
  return invoke("column_create", { request: args });
}

/** Field patch — tri-state per clearable field: OMIT the key to leave the
 * field alone; EXPLICIT null clears it (advanceTo → back to next-column,
 * stepTools → back to unrestricted, step* → unset); a VALUE sets it. The
 * backend distinguishes absent from null, so spreads that materialize
 * undefined keys as null will clear fields. `name` is non-nullable.
 * `isLanding: true` moves the landing flag; `false` on the landing column
 * is rejected (move it instead). */
export function columnUpdate(
  columnId: string,
  patch: {
    name?: string;
    kind?: ColumnKind;
    countsAsDone?: boolean;
    isLanding?: boolean;
    onEnter?: ColumnOnEnter;
    onSettle?: ColumnOnSettle;
    advanceTo?: string | null;
    stepPrompt?: string | null;
    stepProvider?: string | null;
    stepModel?: string | null;
    stepEffort?: string | null;
    stepTools?: string[] | null;
  },
): Promise<BoardColumnDto> {
  return invoke("column_update", { columnId, patch });
}

/** Rejected while the column is occupied (derived from the authoritative
 * lane for seeded columns, from the mirror pointer otherwise) and for the
 * landing column; remaining positions compact to 0..n-1. */
export function columnDelete(columnId: string): Promise<void> {
  return invoke("column_delete", { columnId });
}

/** Full-board reorder: `columnIds` must list every column of the project
 * exactly once, in the new order. Returns the reordered board. */
export function columnReorder(
  projectId: string,
  columnIds: string[],
): Promise<BoardColumnDto[]> {
  return invoke("column_reorder", { projectId, columnIds });
}

// -- E18-04/05 step engine ------------------------------------------------------

/** Launch payload (DispatchOutcomeDto parity: empty prompt + reattached
 * on reattach). Also rides the `step:launch` event. */
export interface StepLaunchInfoDto {
  task: TaskDto;
  prompt: string;
  provider: string;
  model: string | null;
  effort: string | null;
  reattached: boolean;
}

/** What the engine did on entry/confirm. */
export interface EnterOutcomeDto {
  issue: IssueDto;
  /** "launched" | "queued" | "reattached" | "inert" */
  step: "launched" | "queued" | "reattached" | "inert";
  launch: StepLaunchInfoDto | null;
}

/** The engine's move: enter a column, then run/queue/nothing per the
 * column's kind and onEnter. Position omitted = append. Events:
 * step:launch / step:queued ride the fartcode:event channel. */
export function issueEnterColumn(
  issueId: string,
  columnId: string,
  position?: number,
): Promise<EnterOutcomeDto> {
  return invoke("issue_enter_column", { issueId, columnId, position: position ?? null });
}

/** Fires the parked (queue-mode) step. Single-shot: throws "no parked
 * step" when nothing is parked (already confirmed, or cleared by a
 * drag — see step:queue_cleared). */
export function stepConfirm(issueId: string): Promise<EnterOutcomeDto> {
  return invoke("step_confirm", { issueId });
}

/** One parked (queue-mode) step as `step_parked_list` returns it — the
 * same fields the `step:queued` event carries, so the steps store reuses
 * its reducer to rehydrate after a webview reload. */
export interface ParkedStepDto {
  issueId: string;
  projectId: string;
  columnId: string;
  provider: string;
  model: string | null;
  effort: string | null;
}

/** The project's current parks (E18-09 rehydration). Parks live only in
 * the backend's in-memory registry and step:* events cover live sessions
 * only, so a reload re-seeds queued dots / the confirm overlay from this
 * query. Read-only: no state change, no events. */
export function stepParkedList(projectId: string): Promise<ParkedStepDto[]> {
  return invoke("step_parked_list", { projectId });
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

/** Validates a raw fartCode-proposal block payload. Rejects (throws) on
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

// -- E17 dogfood: GitHub issues on the board ----------------------------------

export interface GitHubIssueDto {
  number: number;
  title: string;
  url: string;
  body: string | null;
  labels: string[];
  assignees: string[];
  milestone: string | null;
  createdAt: string | null;
}

/** Open GitHub issues of the project's checkout (gh CLI under the hood). */
export function projectGithubIssues(projectId: string): Promise<GitHubIssueDto[]> {
  return invoke("project_github_issues", { projectId });
}

/** Imports a GitHub issue as a native board card with everything mapped
 * (title, checkbox→acceptance, body, labels/assignees/milestone). The
 * GitHub URL is the dedupe key only — no link survives on the card. */
export function issueImportGithub(args: {
  projectId: string;
  number: number;
  title: string;
  url: string;
  body: string | null;
  labels: string[];
  assignees: string[];
  milestone: string | null;
}): Promise<IssueDto> {
  return invoke("issue_import_github", { request: args });
}

// -- E19-07 memory value dashboard (#76, ADR-0038 item 7) ----------------------
//
// Mirrors fartcode-telemetry's serde output exactly. The tagged enums use
// `kind` with camelCase variant names AND camelCase fields
// (`rename_all_fields`); `TokensSaved::Estimated` is an internally-tagged
// newtype, so EstimatedTokens' fields flatten beside `kind`. The signals
// deliberately carry "don't know" variants — render them, never coerce
// them to numbers.

export interface MemoryCitationsDto {
  sessions: number;
  citedRead: number;
  citedMention: number;
  notCited: number;
  unknown: number;
  unknownWithHit: number;
  wroteWithoutReading: number;
}

export type ReAskRateDto =
  | { kind: "unknown"; stepsScanned: number; stepsUnreadable: number }
  | {
      kind: "observed";
      memoryAnswered: number;
      humanAsked: number;
      stepsTagged: number;
      stepsScanned: number;
      stepsUnreadable: number;
    };

export type TokensSavedDto =
  | {
      kind: "insufficient";
      citingWithUsage: number;
      notCitingWithUsage: number;
      neededPerArm: number;
    }
  | {
      kind: "estimated";
      perSession: number;
      windowTotal: number;
      citingMedian: number;
      notCitingMedian: number;
      citingSessions: number;
      notCitingSessions: number;
      basis: "contextWindowGauge";
    };

export type TimeToLandKindDto =
  | { kind: "noData" }
  | { kind: "singlePoint"; hours: number }
  | {
      kind: "trend";
      earlierMedianHours: number;
      laterMedianHours: number;
      landed: number;
      /** Per-cycle hours in landing order — the sparkline series. */
      landedHours: number[];
    };

/** The caveat is structurally welded to the figures — every payload carries
 * it, for every variant. Never render the kind without it. */
export interface TimeToLandDto {
  caveat: string;
  kind: TimeToLandKindDto;
}

export interface MemoryValueDto {
  projectId: string;
  windowDays: number;
  windowSince: number;
  citations: MemoryCitationsDto;
  reAsk: ReAskRateDto;
  tokensSaved: TokensSavedDto;
  timeToLand: TimeToLandDto;
  sessionsObserved: number;
  dossiersScanned: number;
  cyclesOutsideWindow: number;
  sectionsUndated: number;
  clipped: boolean;
}

/** The four local memory-value signals for one project. Read-only and
 * computed on demand; nothing leaves the machine. */
export function telemetryMemoryValue(
  projectId: string,
  windowDays?: number,
): Promise<MemoryValueDto> {
  return invoke("telemetry_memory_value", { projectId, windowDays });
}
