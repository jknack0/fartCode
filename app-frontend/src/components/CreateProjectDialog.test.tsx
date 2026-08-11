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
  cloneProject: vi.fn(),
  cloneRemoteProject: vi.fn(),
  newRemoteProject: vi.fn(),
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

describe("CreateProjectDialog clone url", () => {
  it("clones locally: url → store cloneProject → close", async () => {
    const cloneProject = vi.fn().mockResolvedValue(undefined);
    useSidebar.setState({ cloneProject });
    const onClose = vi.fn();
    render(<CreateProjectDialog onClose={onClose} />);

    fireEvent.click(screen.getByText("clone url"));
    fireEvent.change(screen.getByLabelText("Repository URL to clone"), {
      target: { value: "git@github.com:acme/api.git" },
    });
    fireEvent.click(screen.getByText("clone project"));
    await waitFor(() =>
      expect(cloneProject).toHaveBeenCalledWith("git@github.com:acme/api.git"),
    );
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it("clones on the host: remote tab + url → cloneRemoteProject with the connection", async () => {
    const cloneRemoteProject = vi.fn().mockResolvedValue(undefined);
    useSidebar.setState({ cloneRemoteProject });
    render(<CreateProjectDialog onClose={() => {}} />);

    fireEvent.click(screen.getByRole("tab", { name: "remote · ssh" }));
    await waitFor(() => expect(vi.mocked(sshConnectionList)).toHaveBeenCalled());
    fireEvent.click(screen.getByText("clone url"));
    fireEvent.change(screen.getByLabelText("Repository URL to clone"), {
      target: { value: "https://github.com/acme/api" },
    });
    fireEvent.click(screen.getByText("clone on host"));
    await waitFor(() =>
      expect(cloneRemoteProject).toHaveBeenCalledWith(
        "c1",
        "https://github.com/acme/api",
      ),
    );
  });
});

describe("CreateProjectDialog new repo", () => {
  it("is remote-only and inits on the host", async () => {
    const newRemoteProject = vi.fn().mockResolvedValue(undefined);
    useSidebar.setState({ newRemoteProject });
    const onClose = vi.fn();
    render(<CreateProjectDialog onClose={onClose} />);

    // Local tab: the pill does not exist.
    expect(screen.queryByText("new repo")).toBeNull();

    fireEvent.click(screen.getByRole("tab", { name: "remote · ssh" }));
    await waitFor(() => expect(vi.mocked(sshConnectionList)).toHaveBeenCalled());
    fireEvent.click(screen.getByText("new repo"));
    fireEvent.change(screen.getByLabelText("New repository name"), {
      target: { value: "my app" },
    });
    fireEvent.click(screen.getByText("create on host"));
    await waitFor(() =>
      expect(newRemoteProject).toHaveBeenCalledWith("c1", "my app"),
    );
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it("falls back to pick when switching to the local tab mid-new", async () => {
    render(<CreateProjectDialog onClose={() => {}} />);
    fireEvent.click(screen.getByRole("tab", { name: "remote · ssh" }));
    await waitFor(() => expect(vi.mocked(sshConnectionList)).toHaveBeenCalled());
    fireEvent.click(screen.getByText("new repo"));
    expect(screen.getByLabelText("New repository name")).toBeTruthy();

    fireEvent.click(screen.getByRole("tab", { name: "local" }));
    // Not a dead "new" form pointing at no host — the local path input.
    expect(screen.queryByLabelText("New repository name")).toBeNull();
    expect(screen.getByLabelText("Path to a local git repository")).toBeTruthy();
  });
});
