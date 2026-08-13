// Host-dependency store (7d "Agents on this machine", E3-02): detection
// rows + registry tail counts, shared between App settings and onboarding
// step two. Install/update are tracked per provider id; the backend's
// install runner reports no progress events, so "installing" renders bare.
import { create } from "zustand";
import {
  DependencyRegistrySummaryDto,
  HostDependencyDto,
  hostDependencyInstall,
  hostDependencyList,
  hostDependencyRegistrySummary,
  hostDependencyUpdate,
  onDependencyOutput,
} from "../lib/tauri";
import { wireEvents } from "../lib/wireEvents";

interface DependenciesState {
  deps: HostDependencyDto[];
  summary: DependencyRegistrySummaryDto | null;
  loading: boolean;
  error: string | null;
  /** providerIds with an install/update in flight. */
  installing: Record<string, boolean>;
  /** Live installer output tail per providerId (rendered by the row). */
  progress: Record<string, string>;
  /** Loads rows + registry summary; refresh forces a re-detect. */
  load: (refresh?: boolean) => Promise<void>;
  install: (providerId: string) => Promise<void>;
  update: (providerId: string) => Promise<void>;
}

function replaceRow(deps: HostDependencyDto[], row: HostDependencyDto): HostDependencyDto[] {
  return deps.map((d) => (d.providerId === row.providerId ? row : d));
}

export const useDependencies = create<DependenciesState>((set) => ({
  deps: [],
  summary: null,
  loading: false,
  error: null,
  installing: {},
  progress: {},

  load: async (refresh = false) => {
    set({ loading: true, error: null });
    try {
      const [deps, summary] = await Promise.all([
        hostDependencyList(refresh),
        hostDependencyRegistrySummary(),
      ]);
      set({ deps, summary, loading: false });
    } catch (e) {
      set({ error: String(e), loading: false });
    }
  },

  install: async (providerId) => {
    set((s) => ({
      installing: { ...s.installing, [providerId]: true },
      progress: { ...s.progress, [providerId]: "" },
      error: null,
    }));
    try {
      const row = await hostDependencyInstall(providerId);
      set((s) => ({ deps: replaceRow(s.deps, row) }));
    } catch (e) {
      set({ error: String(e) });
    } finally {
      set((s) => {
        const installing = { ...s.installing };
        delete installing[providerId];
        return { installing };
      });
    }
  },

  update: async (providerId) => {
    set((s) => ({
      installing: { ...s.installing, [providerId]: true },
      progress: { ...s.progress, [providerId]: "" },
      error: null,
    }));
    try {
      const row = await hostDependencyUpdate(providerId);
      set((s) => ({ deps: replaceRow(s.deps, row) }));
    } catch (e) {
      set({ error: String(e) });
    } finally {
      set((s) => {
        const installing = { ...s.installing };
        delete installing[providerId];
        return { installing };
      });
    }
  },
}));

/** The default agent's display name ("claude" fallback) — 7c Agent group. */
export function defaultAgentName(deps: HostDependencyDto[]): string {
  return deps.find((d) => d.isDefault)?.name ?? "claude";
}

/** App-lifetime wiring: `dependency:output` → per-provider live row tail. */
export function wireDependencyEvents(): () => void {
  return wireEvents(onDependencyOutput, ({ providerId, data }) => {
    useDependencies.setState((s) => {
      const prev = s.progress[providerId] ?? "";
      // Chatty installers write a lot; the row only shows the last frame,
      // so a small bounded tail is enough.
      const next = (prev + data).slice(-8192);
      return { progress: { ...s.progress, [providerId]: next } };
    });
  });
}
