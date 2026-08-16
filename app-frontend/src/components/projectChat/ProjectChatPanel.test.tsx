// #126: the PM chat resolves its provider from the defaultAgent app
// setting (registry-first-ACP fallback) and names provider · model in the
// header. The three shapes: setting names an ACP provider (honored),
// setting names a non-ACP/unknown provider (fallback), no ACP provider at
// all (the error is live, not dead code).

import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";

const { ensureProject } = vi.hoisted(() => ({
  ensureProject: vi.fn(() => Promise.resolve({ id: "conv-1" })),
}));

vi.mock("../../lib/tauri", () => ({
  listProviders: vi.fn(() => Promise.resolve([])),
  getAppSetting: vi.fn(() => Promise.resolve(null)),
  acpStart: vi.fn(() => Promise.resolve()),
}));
vi.mock("../../store/conversations", () => ({
  useConversations: { getState: () => ({ ensureProject }) },
}));
vi.mock("../../lib/useCommands", () => ({ hint: () => "⌘⇧2" }));
vi.mock("../ConversationView", () => ({
  default: () => <div data-testid="conversation-view" />,
}));

import ProjectChatPanel from "./ProjectChatPanel";
import { getAppSetting, listProviders } from "../../lib/tauri";
import type { ProviderDto } from "../../lib/tauri";

function provider(over: Partial<ProviderDto>): ProviderDto {
  return {
    id: "claude",
    name: "Claude",
    description: "",
    websiteUrl: null,
    capabilities: ["acp"],
    models: [],
    defaultModel: null,
    binaries: [],
    promptStrategy: "acp",
    authMethods: [],
    ...over,
  };
}

const claude = provider({ id: "claude", name: "Claude", defaultModel: "sonnet" });
const gemini = provider({ id: "gemini", name: "Gemini", defaultModel: "gemini-pro" });
const rovo = provider({ id: "rovo", name: "Rovo", capabilities: [] });

beforeEach(() => {
  vi.clearAllMocks();
});

describe("ProjectChatPanel provider resolution (#126)", () => {
  it("honors an ACP-capable defaultAgent and names it in the header", async () => {
    vi.mocked(listProviders).mockResolvedValue([claude, gemini]);
    vi.mocked(getAppSetting).mockResolvedValue("gemini");
    render(<ProjectChatPanel projectId="p1" />);
    await waitFor(() => expect(ensureProject).toHaveBeenCalledWith("p1", "gemini"));
    expect(screen.getByText(/Gemini · gemini-pro · project root/)).toBeInTheDocument();
  });

  it("falls back to the first ACP registry entry when defaultAgent is not ACP-capable", async () => {
    vi.mocked(listProviders).mockResolvedValue([rovo, claude, gemini]);
    vi.mocked(getAppSetting).mockResolvedValue("rovo");
    render(<ProjectChatPanel projectId="p1" />);
    await waitFor(() => expect(ensureProject).toHaveBeenCalledWith("p1", "claude"));
    expect(screen.getByText(/Claude · sonnet · project root/)).toBeInTheDocument();
  });

  it("surfaces the no-ACP-provider error instead of starting", async () => {
    vi.mocked(listProviders).mockResolvedValue([rovo]);
    render(<ProjectChatPanel projectId="p1" />);
    await waitFor(() =>
      expect(
        screen.getByText(/no ACP-capable provider available/),
      ).toBeInTheDocument(),
    );
    expect(ensureProject).not.toHaveBeenCalled();
  });
});
