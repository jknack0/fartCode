// The warning strip exists for exactly one reason: a failed BYOI terminate
// is billed money, and tracing::warn reaches nobody. Worth pinning: the
// strip renders nothing until an event arrives, and dismiss removes ONE
// warning, not the strip.
import { describe, it, expect, vi } from "vitest";
import { render, screen, act, fireEvent } from "@testing-library/react";
import type { FartcodeEvent } from "../lib/tauri";

let emit: (event: FartcodeEvent) => void = () => {};

vi.mock("../lib/tauri", () => ({
  onFartcodeEvent: vi.fn((cb: (e: FartcodeEvent) => void) => {
    emit = cb;
    return Promise.resolve(() => {});
  }),
}));

import Warnings from "./Warnings";

const warning = (message: string): FartcodeEvent => ({
  type: "task:terminate_warning",
  taskId: "t1",
  message,
});

describe("Warnings", () => {
  it("renders nothing until a terminate warning arrives, then alerts", async () => {
    const { container } = render(<Warnings />);
    await act(async () => {});
    expect(container.firstChild).toBeNull();

    act(() => emit(warning("terminate script exited 1 — machine may still be running")));
    expect(screen.getByRole("alert").textContent).toContain("exited 1");
  });

  it("dismisses one warning, keeps the rest", async () => {
    render(<Warnings />);
    await act(async () => {});
    act(() => {
      emit(warning("first leak"));
      emit(warning("second leak"));
    });
    expect(screen.getAllByRole("alert")).toHaveLength(2);

    fireEvent.click(screen.getAllByText("dismiss")[0]);
    const left = screen.getAllByRole("alert");
    expect(left).toHaveLength(1);
    expect(left[0].textContent).toContain("second leak");
  });

  it("ignores unrelated events", async () => {
    const { container } = render(<Warnings />);
    await act(async () => {});
    act(() => emit({ type: "task:deleted", taskId: "t1" }));
    expect(container.firstChild).toBeNull();
  });
});
