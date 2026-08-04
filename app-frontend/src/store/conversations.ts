// Conversation state (E2-08): list, create, delete, active conversation,
// message input, context items.

import { create } from "zustand";
import {
  listConversations,
  createConversation as apiCreateConversation,
  deleteConversation as apiDeleteConversation,
  type ConversationDto,
} from "../lib/tauri";

export type ContextItem =
  | { kind: "file"; path: string }
  | { kind: "prompt"; text: string };

interface ConversationState {
  conversations: ConversationDto[];
  activeId: string | null;
  draftPrompt: string;
  contextItems: ContextItem[];

  load: (taskId: string) => Promise<void>;
  create: (
    taskId: string,
    projectId: string,
    provider?: string,
    title?: string,
    model?: string,
  ) => Promise<ConversationDto>;
  remove: (projectId: string, taskId: string, id: string) => Promise<void>;
  setActive: (id: string | null) => void;
  setDraftPrompt: (text: string) => void;
  addContextFile: (path: string) => void;
  addContextPrompt: (text: string) => void;
  removeContextItem: (index: number) => void;
  clearContext: () => void;
}

export const useConversations = create<ConversationState>((set) => ({
  conversations: [],
  activeId: null,
  draftPrompt: "",
  contextItems: [],

  load: async (taskId) => {
    const convs = await listConversations(taskId);
    set((s) => ({
      conversations: convs,
      activeId: s.activeId && convs.find((c) => c.id === s.activeId)
        ? s.activeId
        : convs[0]?.id ?? null,
    }));
  },

  create: async (taskId, projectId, provider, title, model) => {
    const created = await apiCreateConversation(
      projectId,
      taskId,
      provider ?? null,
      title ?? "New conversation",
      model ?? null,
      null,
    );
    set((s) => ({
      conversations: [...s.conversations, created],
      activeId: created.id,
    }));
    return created;
  },

  remove: async (projectId, taskId, id) => {
    await apiDeleteConversation(projectId, taskId, id);
    set((s) => {
      const conversations = s.conversations.filter((c) => c.id !== id);
      return {
        conversations,
        activeId: s.activeId === id ? (conversations[0]?.id ?? null) : s.activeId,
      };
    });
  },

  setActive: (id) => set({ activeId: id }),
  setDraftPrompt: (text) => set({ draftPrompt: text }),
  addContextFile: (path) =>
    set((s) => ({
      contextItems: [...s.contextItems, { kind: "file" satisfies ContextItem["kind"], path }],
    })),
  addContextPrompt: (text) =>
    set((s) => ({
      contextItems: [...s.contextItems, { kind: "prompt" satisfies ContextItem["kind"], text }],
    })),
  removeContextItem: (index) =>
    set((s) => ({
      contextItems: s.contextItems.filter((_, i) => i !== index),
    })),
  clearContext: () => set({ contextItems: [], draftPrompt: "" }),
}));
