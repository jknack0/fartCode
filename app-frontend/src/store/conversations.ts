// Conversation state (E2-11-5): the task's conversations with their
// provider-decided runtime (`"tui"` | `"acp"`), plus the live ACP model
// snapshots the backend streams over `acp:transcript`.
//
// Terminal-only UI note: this store is the routing seam, not a renderer.
// ⌘Enter (add-and-send-context) reads `activeAcp(taskId)` to pick the ACP
// path; #33 builds the transcript UI on top of `models`.
import { create } from "zustand";
import {
  acpCancel,
  acpHistory,
  acpResolvePermission,
  acpSendPrompt,
  acpStart,
  acpStop,
  createConversation,
  listConversations,
  onAcpPermissionRequest,
  onAcpTranscript,
  onAcpUpdate,
  onAdeEvent,
  type ConversationDto,
  type LiveModels,
  type PendingPermission,
} from "../lib/tauri";

/** A surfaced permission prompt awaiting a user decision. */
export interface PermissionPrompt {
  conversationId: string;
  requestId: string;
  pending: PendingPermission;
}

interface ConversationsState {
  /** Conversations per task, in creation order. */
  byTask: Record<string, ConversationDto[]>;
  /** Latest live-model snapshot per conversation id (acp:transcript). */
  models: Record<string, LiveModels>;
  /** Raw update counter per conversation id (streaming liveness signal). */
  updateCounts: Record<string, number>;
  /** Permission prompts awaiting a decision, per conversation id. */
  permissions: Record<string, PermissionPrompt[]>;
  /** Composer draft per conversation id (local until the composer lands). */
  drafts: Record<string, string>;

  /** Loads the task's conversations (idempotent per task). */
  ensure: (taskId: string) => Promise<void>;
  /** Creates a conversation; the runtime type is decided server-side. */
  create: (
    projectId: string,
    taskId: string,
    provider?: string | null,
    title?: string,
  ) => Promise<ConversationDto>;
  /** The task's ACP conversation, if any (⌘Enter routing). */
  activeAcp: (taskId: string) => ConversationDto | null;
  /** Starts the session lazily, then sends the prompt (⌘Enter path). */
  sendPrompt: (conversationId: string, text: string) => Promise<{ queued: boolean }>;
  /** Cancels the in-flight turn (composer Stop — the session stays up). */
  cancel: (conversationId: string) => Promise<void>;
  stop: (conversationId: string) => Promise<void>;
  /** Loads the reduced history snapshot when no live one arrived yet
   * (ConversationView mount — restart/restore path). No-op when a
   * snapshot exists or the session is not running (`acp_history` null). */
  hydrate: (conversationId: string) => Promise<void>;
  resolvePermission: (
    conversationId: string,
    requestId: string,
    optionId: string,
  ) => Promise<void>;
  setDraft: (conversationId: string, text: string) => void;
  /** Task-deletion teardown mirror of the tabs store's dropTask. */
  dropTask: (taskId: string) => void;
}

export const useConversations = create<ConversationsState>((set, get) => ({
  byTask: {},
  models: {},
  updateCounts: {},
  permissions: {},
  drafts: {},

  ensure: async (taskId) => {
    if (get().byTask[taskId]) return;
    const list = await listConversations(taskId);
    set((s) => ({ byTask: { ...s.byTask, [taskId]: list } }));
  },

  create: async (projectId, taskId, provider = null, title) => {
    const conv = await createConversation(projectId, taskId, provider, title);
    set((s) => ({
      byTask: { ...s.byTask, [taskId]: [...(s.byTask[taskId] ?? []), conv] },
    }));
    return conv;
  },

  activeAcp: (taskId) =>
    (get().byTask[taskId] ?? []).find((c) => c.runtime === "acp") ?? null,

  sendPrompt: async (conversationId, text) => {
    // Lazy start: the session may not be running yet (first ⌘Enter after
    // boot). acp_start is idempotent, so racing sends converge.
    await acpStart(conversationId);
    const response = await acpSendPrompt(conversationId, text);
    set((s) => ({ drafts: { ...s.drafts, [conversationId]: "" } }));
    return response;
  },

  cancel: async (conversationId) => {
    await acpCancel(conversationId);
  },

  hydrate: async (conversationId) => {
    if (get().models[conversationId] || hydrating.has(conversationId)) return;
    hydrating.add(conversationId);
    try {
      const models = await acpHistory(conversationId);
      if (models) applySnapshot(conversationId, models);
    } finally {
      hydrating.delete(conversationId);
    }
  },

  stop: async (conversationId) => {
    await acpStop(conversationId);
  },

  resolvePermission: async (conversationId, requestId, optionId) => {
    set((s) => ({
      permissions: {
        ...s.permissions,
        [conversationId]: (s.permissions[conversationId] ?? []).filter(
          (p) => p.requestId !== requestId,
        ),
      },
    }));
    // The backend re-emits the full snapshot on resolution; the optimistic
    // removal above keeps the prompt card snappy.
    await acpResolvePermission(conversationId, requestId, optionId);
  },

  setDraft: (conversationId, text) =>
    set((s) => ({ drafts: { ...s.drafts, [conversationId]: text } })),

  dropTask: (taskId) => {
    const conversations = get().byTask[taskId] ?? [];
    set((s) => {
      const byTask = { ...s.byTask };
      const models = { ...s.models };
      const updateCounts = { ...s.updateCounts };
      const permissions = { ...s.permissions };
      const drafts = { ...s.drafts };
      delete byTask[taskId];
      for (const conv of conversations) {
        delete models[conv.id];
        delete updateCounts[conv.id];
        delete permissions[conv.id];
        delete drafts[conv.id];
      }
      return { byTask, models, updateCounts, permissions, drafts };
    });
  },
}));

/** In-flight hydrate guard (per conversation id). */
const hydrating = new Set<string>();

/** Applies one live-model snapshot (the acp:transcript payload). */
function applySnapshot(conversationId: string, models: LiveModels): void {
  // Sync surfaced permission prompts from the snapshot so a store reload
  // (task switch and back) never orphans a pending prompt.
  const prompts: PermissionPrompt[] = models.sessionState.pendingPermissions.map(
    (pending) => ({ conversationId, requestId: pending.requestId, pending }),
  );
  useConversations.setState((s) => ({
    models: { ...s.models, [conversationId]: models },
    permissions: { ...s.permissions, [conversationId]: prompts },
  }));
}

/**
 * Subscribes to the ACP event channels once (App startup); mirrors
 * wireTabsEvents. Returns the combined unsubscribe.
 */
export function wireConversationEvents(): () => void {
  const unsubs: Array<() => void> = [];
  let disposed = false;

  void onAcpTranscript(({ conversationId, models }) => {
    if (!disposed) applySnapshot(conversationId, models);
  }).then((un) => unsubs.push(un));

  void onAcpUpdate(({ conversationId }) => {
    if (disposed) return;
    useConversations.setState((s) => ({
      updateCounts: {
        ...s.updateCounts,
        [conversationId]: (s.updateCounts[conversationId] ?? 0) + 1,
      },
    }));
  }).then((un) => unsubs.push(un));

  void onAcpPermissionRequest(({ conversationId, requestId, pending }) => {
    if (disposed) return;
    useConversations.setState((s) => {
      const existing = s.permissions[conversationId] ?? [];
      if (existing.some((p) => p.requestId === requestId)) return s;
      return {
        permissions: {
          ...s.permissions,
          [conversationId]: [...existing, { conversationId, requestId, pending }],
        },
      };
    });
  }).then((un) => unsubs.push(un));

  // Task deletion drops the task's conversation state (the backend stops
  // the sessions during delete_task teardown).
  void onAdeEvent((event) => {
    if (event.type === "task:deleted") {
      useConversations.getState().dropTask(event.taskId);
    }
  }).then((un) => unsubs.push(un));

  return () => {
    disposed = true;
    for (const un of unsubs) un();
  };
}

// Browser test seam (ade-frontend-browser-smoke): the app has no frontend
// test runner, so mocked-backend verification asserts store mutations
// through this global. Dev/build only — the production webview never
// reads it.
declare global {
  interface Window {
    __conversationsStore?: typeof useConversations;
  }
}
if (typeof window !== "undefined") window.__conversationsStore = useConversations;
