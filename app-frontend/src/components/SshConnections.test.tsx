// SSH connections panel (E12-06 lifecycle in the UI).
//
// The two things worth pinning: a reconnect frame has to render the ladder's
// own numbers (the backoff is the backend's, not the panel's), and a degraded
// health frame has to read as MaxSessions ADVICE rather than a failure — the
// session is alive, so “reconnect” is the wrong suggestion.
import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, waitFor, act } from "@testing-library/react";
import type { FartcodeEvent } from "../lib/tauri";

let emit: (event: FartcodeEvent) => void = () => {};

vi.mock("../lib/tauri", () => ({
  sshConnectionList: vi.fn(),
  sshConnectionStates: vi.fn(),
  sshConnectionSave: vi.fn(),
  sshConnectionDelete: vi.fn(),
  sshConnect: vi.fn(),
  sshDisconnect: vi.fn(),
  onFartcodeEvent: vi.fn((cb: (e: FartcodeEvent) => void) => {
    emit = cb;
    return Promise.resolve(() => {});
  }),
}));

import SshConnections, { stateSummary } from "./SshConnections";
import { sshConnectionList, sshConnectionStates } from "../lib/tauri";

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

beforeEach(() => {
  vi.mocked(sshConnectionList).mockResolvedValue([CONNECTION]);
  vi.mocked(sshConnectionStates).mockResolvedValue([
    { connectionId: "c1", state: "connected", connected: true, degraded: false },
  ]);
});

describe("SshConnections", () => {
  it("shows the connection and its live state", async () => {
    render(<SshConnections />);
    await waitFor(() => expect(screen.getByText("build box")).toBeTruthy());
    expect(screen.getByText("connected")).toBeTruthy();
    expect(screen.getByText("disconnect")).toBeTruthy();
  });

  it("counts down the backoff ladder from the event, not its own timer", async () => {
    render(<SshConnections />);
    await waitFor(() => expect(screen.getByText("build box")).toBeTruthy());

    act(() => {
      emit({
        type: "ssh:state_changed",
        connectionId: "c1",
        state: "reconnecting",
        attempt: 3,
        delayMs: 5000,
        error: null,
      });
    });

    await waitFor(() =>
      expect(screen.getByText("reconnecting · 5s (3/5)")).toBeTruthy(),
    );
    // Reconnecting is not connected: the action offers to dial, not to drop.
    expect(screen.getByText("connect")).toBeTruthy();
  });

  it("raises the MaxSessions note on a degraded host", async () => {
    render(<SshConnections />);
    await waitFor(() => expect(screen.getByText("build box")).toBeTruthy());
    expect(screen.queryByText("MaxSessions 100")).toBeNull();

    act(() => {
      emit({ type: "ssh:health_changed", connectionId: "c1", degraded: true });
    });

    await waitFor(() =>
      expect(screen.getByText("MaxSessions 100")).toBeTruthy(),
    );
    expect(
      screen.getByText(/build box refused a new channel/),
    ).toBeTruthy();

    act(() => {
      emit({ type: "ssh:health_changed", connectionId: "c1", degraded: false });
    });
    await waitFor(() =>
      expect(screen.queryByText("MaxSessions 100")).toBeNull(),
    );
  });
});

describe("stateSummary", () => {
  it("names states in the user's words, not the enum's", () => {
    const base = { attempt: null, delayMs: null, error: null, degraded: false };
    expect(stateSummary({ ...base, state: "disconnected" })).toBe("offline");
    expect(stateSummary({ ...base, state: "error" })).toBe("unreachable");
    expect(stateSummary({ ...base, state: "connecting" })).toBe("connecting");
  });

  it("falls back to the plain label when the ladder has no numbers", () => {
    expect(
      stateSummary({
        state: "reconnecting",
        attempt: null,
        delayMs: null,
        error: null,
        degraded: false,
      }),
    ).toBe("reconnecting");
  });
});
