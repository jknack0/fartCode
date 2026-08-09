// Vitest setup: jest-dom matchers (this import also augments vitest's
// Assertion types, so tsconfig needs no `types` entry) plus RTL teardown.
// Tests import { describe, it, expect } from "vitest" explicitly — globals
// are off, so React Testing Library's auto-cleanup never registers itself
// and we wire it here instead.
import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

// jsdom does no layout, so it ships no scrollIntoView. Anything that keeps
// keyboard focus visible calls it (the board's card focus and narrow
// strip, the flyout); a no-op keeps those effects from throwing.
if (!Element.prototype.scrollIntoView) {
  Element.prototype.scrollIntoView = () => {};
}

afterEach(cleanup);
