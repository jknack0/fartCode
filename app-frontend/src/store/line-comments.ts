// Line-comment store (E4-10, ARCHITECTURE.md §14): per-task comment lists
// keyed by taskId, loaded when a diff tab opens (listLineComments) and kept
// live via `comment:created` / `comment:resolved` bus events. Gutter
// markers and the comment popover read from here; nothing polls.
import { create } from "zustand";
import {
  addLineComment,
  deleteLineComment,
  listLineComments,
  onFartcodeEvent,
  resolveLineComment,
  type LineCommentDto,
} from "../lib/tauri";

interface LineCommentsState {
  byTask: Record<string, LineCommentDto[]>;
  /** Loads (or reloads) a task's comments — called when a diff opens. */
  ensure: (taskId: string) => Promise<void>;
  addNote: (args: {
    taskId: string;
    filePath: string;
    lineNumber: number;
    lineEnd: number | null;
    sourceSide: "before" | "after";
    lineContent: string | null;
    content: string;
  }) => Promise<LineCommentDto>;
  resolve: (commentId: string) => Promise<void>;
  remove: (commentId: string) => Promise<void>;
  /** Replaces the linked task id locally (create-task flow). */
  markLinked: (commentId: string, taskId: string) => void;
}

const inflight = new Set<string>();

export const useLineComments = create<LineCommentsState>((set, get) => {
  const patchTask = (taskId: string, comments: LineCommentDto[]) =>
    set((s) => ({ byTask: { ...s.byTask, [taskId]: comments } }));

  const patchComment = (updated: LineCommentDto) =>
    set((s) => {
      const list = s.byTask[updated.taskId];
      if (!list) return s;
      return {
        byTask: {
          ...s.byTask,
          [updated.taskId]: list.map((c) => (c.id === updated.id ? updated : c)),
        },
      };
    });

  return {
    byTask: {},

    ensure: async (taskId) => {
      if (inflight.has(taskId)) return;
      inflight.add(taskId);
      try {
        const comments = await listLineComments(taskId);
        patchTask(taskId, comments);
      } catch {
        // A task without comments support (missing workspace) keeps an
        // empty list rather than an error state — the gutter simply shows
        // nothing.
        if (!(taskId in get().byTask)) patchTask(taskId, []);
      } finally {
        inflight.delete(taskId);
      }
    },

    addNote: async (args) => {
      const comment = await addLineComment(args);
      // Optimistic local insert — the comment:created event also lands, but
      // this keeps the marker visible immediately.
      const list = get().byTask[comment.taskId] ?? [];
      if (!list.some((c) => c.id === comment.id)) {
        patchTask(comment.taskId, [...list, comment]);
      }
      return comment;
    },

    resolve: async (commentId) => {
      const updated = await resolveLineComment(commentId);
      patchComment(updated);
    },

    remove: async (commentId) => {
      await deleteLineComment(commentId);
      set((s) => {
        const byTask: Record<string, LineCommentDto[]> = {};
        for (const [taskId, list] of Object.entries(s.byTask)) {
          byTask[taskId] = list.filter((c) => c.id !== commentId);
        }
        return { byTask };
      });
    },

    markLinked: (commentId, taskId) =>
      set((s) => {
        const byTask: Record<string, LineCommentDto[]> = {};
        let changed = false;
        for (const [tid, list] of Object.entries(s.byTask)) {
          byTask[tid] = list.map((c) => {
            if (c.id !== commentId) return c;
            changed = true;
            return { ...c, linkedTaskId: taskId };
          });
        }
        return changed ? { byTask } : s;
      }),
  };
});

/** Test seam (browser smoke), mirrors the other stores. */
declare global {
  interface Window {
    __lineCommentsStore?: typeof useLineComments;
  }
}
if (typeof window !== "undefined") window.__lineCommentsStore = useLineComments;

/** Bus wiring: created → insert, resolved → flip flag. Installed once in
 * App alongside the other store wirings. */
export function wireLineCommentEvents(): () => void {
  let unlisten: (() => void) | null = null;
  let disposed = false;
  void onFartcodeEvent((event) => {
    const s = useLineComments.getState();
    if (event.type === "comment:created") {
      // The adding side already patched its own list; other open diffs of
      // the same task reload so their gutters match.
      if (event.taskId in s.byTask) void s.ensure(event.taskId);
    } else if (event.type === "comment:resolved") {
      for (const [taskId, list] of Object.entries(s.byTask)) {
        const found = list.find((c) => c.id === event.id);
        if (found && !found.resolved) {
          useLineComments.setState((st) => ({
            byTask: {
              ...st.byTask,
              [taskId]: st.byTask[taskId].map((c) =>
                c.id === event.id ? { ...c, resolved: true } : c,
              ),
            },
          }));
        }
      }
    }
  }).then((off) => {
    if (disposed) off();
    else unlisten = off;
  });
  return () => {
    disposed = true;
    unlisten?.();
  };
}
