import { describe, expect, it } from "vitest";
import { errorContext, isErrorLine, parseLog } from "./model";

describe("isErrorLine", () => {
  it("flags failures", () => {
    expect(isErrorLine("FAIL src/login.test.ts")).toBe(true);
    expect(isErrorLine("Tests: 1 failed, 41 passed")).toBe(true);
    expect(isErrorLine("thread 'main' panicked at src/lib.rs")).toBe(true);
  });

  it("ignores zero counts", () => {
    expect(isErrorLine("Tests: 0 failed, 42 passed")).toBe(false);
    expect(isErrorLine("errors: 0, warnings: 3")).toBe(false);
    expect(isErrorLine("0 errors, 1 failed")).toBe(true);
  });
});

describe("parseLog", () => {
  it("turns markers into groups and kinds", () => {
    const { lines, groups } = parseLog(
      ["##[group]Run npm ci", "npm ci", "##[endgroup]", "[command]/usr/bin/git version", "+ make test", "npm WARN old", "##[error]exit 1"].join("\n"),
    );
    expect(groups).toEqual([{ id: 0, title: "Run npm ci", n: 1, size: 1, hasError: false }]);
    expect(lines.map((l) => [l.n, l.kind, l.text, l.group])).toEqual([
      [2, "plain", "npm ci", 0],
      [4, "command", "/usr/bin/git version", null],
      [5, "command", "+ make test", null],
      [6, "warning", "npm WARN old", null],
      [7, "error", "exit 1", null],
    ]);
  });

  it("marks groups holding errors and survives unbalanced markers", () => {
    const { lines, groups } = parseLog(["##[endgroup]", "##[group]A", "##[group]B", "boom error", "tail"].join("\n"));
    expect(groups.map((g) => [g.title, g.size, g.hasError])).toEqual([
      ["A", 0, false],
      ["B", 2, true],
    ]);
    expect(lines.map((l) => l.group)).toEqual([1, 1]);
  });
});

describe("errorContext", () => {
  const lines = parseLog(Array.from({ length: 20 }, (_, i) => (i === 3 || i === 15 ? "error here" : `line ${i}`)).join("\n")).lines;

  it("keeps context around each error and marks gaps", () => {
    expect(errorContext(lines, 1)).toEqual([2, 3, 4, null, 14, 15, 16]);
  });

  it("merges overlapping windows", () => {
    expect(errorContext(lines, 6)).toEqual([...Array(20).keys()]);
  });
});
