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
  | { type: "task:status_changed"; taskId: string; status: string };

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
