// Vitest setup: jest-dom matchers (this import also augments vitest's
// Assertion types, so tsconfig needs no `types` entry) plus RTL teardown.
// Tests import { describe, it, expect } from "vitest" explicitly — globals
// are off, so React Testing Library's auto-cleanup never registers itself
// and we wire it here instead.
import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

afterEach(cleanup);
