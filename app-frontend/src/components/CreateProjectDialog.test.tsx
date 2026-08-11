// Remote project picker (E12-04): the dialog's remote tab is the first
// caller `remote_browse` ever had. Worth pinning: the browser walks by
// clicking directories (each click is a fresh listing, not client state),
// and "add remote project" hands the CURRENT directory to the store.
import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("../lib/useCommands", () => ({ hint: () => "" }));
vi.mock("../lib/tauri", () => ({
  sshConnectionList: vi.fn(),
  remoteBrowse: vi.fn(),
  createProject: vi.fn(),
  createRemoteProject: vi.fn(),
  onFartcodeEvent: vi.fn(() => Promise.resolve(() => {})),
}));

import { CreateProjectDialog } from "./Modals";
import { remoteBrowse, sshConnectionList } from "../lib/tauri";
import { useSidebar } from "../store/sidebar";

const CONNECTION = {
  id: "c1",
  name: "build box",
  host: "10.0.0.4",
  port: 22,
  username: "deploy",
  authType: "agent" as const,
  privateKeyPath: null,
  useAgent: true,
  alias: null,
  proxyJump: null,
  forwardAgent: false,
  projectsDirectory: null,
  hasSecret: false,
};

const dir = (path: string) => ({
  path,
  name: path.split("/").pop() ?? path,
  kind: "dir" as const,
});

beforeEach(() => {
  vi.mocked(sshConnectionList).mockResolvedValue([CONNECTION]);
  vi.mocked(remoteBrowse).mockResolvedValue([
    dir("/home/deploy/src"),
    dir("/home/deploy/api"),
  ]);
});

describe("CreateProjectDialog remote tab", () => {
  it("lists the login dir, walks into a directory, and adds it", async () => {
    const createRemoteProject = vi.fn().mockResolvedValue(undefined);
    useSidebar.setState({ createRemoteProject });
    const onClose = vi.fn();
    render(<CreateProjectDialog onClose={onClose} />);

    fireEvent.click(screen.getByRole("tab", { name: "remote · ssh" }));

    // First listing: no path — the backend starts at the host's login dir;
    // the dialog recovers the cwd from an entry's parent.
    await waitFor(() => expect(screen.getByText("src/")).toBeTruthy());
    expect(vi.mocked(remoteBrowse)).toHaveBeenCalledWith("c1", undefined);
    expect(screen.getByLabelText("Remote repository path")).toHaveProperty(
      "value",
      "/home/deploy",
    );

    // Clicking a directory is a fresh listing rooted there.
    vi.mocked(remoteBrowse).mockResolvedValue([]);
    fireEvent.click(screen.getByText("src/"));
    await waitFor(() =>
      expect(vi.mocked(remoteBrowse)).toHaveBeenCalledWith(
        "c1",
        "/home/deploy/src",
      ),
    );
    await waitFor(() => expect(screen.getByText("no subdirectories")).toBeTruthy());

    // Add hands the CURRENT directory to the store, then closes.
    fireEvent.click(screen.getByText("add remote project"));
    await waitFor(() =>
      expect(createRemoteProject).toHaveBeenCalledWith("c1", "/home/deploy/src"),
    );
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it("disables add until a host and directory are known", async () => {
    vi.mocked(sshConnectionList).mockResolvedValue([]);
    render(<CreateProjectDialog onClose={() => {}} />);
    fireEvent.click(screen.getByRole("tab", { name: "remote · ssh" }));
    await waitFor(() =>
      expect(screen.getByText("no connections — add one in settings")).toBeTruthy(),
    );
    const add = screen.getByText("add remote project").closest("button");
    expect(add?.disabled).toBe(true);
  });
});
