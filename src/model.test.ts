import { describe, expect, it } from "vitest";
import { contextRows, errorContext, formatSecs, isErrorLine, newLines, normaliseLine, parseLog } from "./model";

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

describe("comparing with the last pass", () => {
  it("ignores timings, ids and hashes", () => {
    expect(normaliseLine("Done in 14.2s (build 7f3a9c1e)")).toBe(normaliseLine("Done in 9.8s (build 0b12ffe9)"));
    expect(normaliseLine("FAIL src/login.test.ts")).not.toBe(normaliseLine("PASS src/login.test.ts"));
  });

  it("finds lines the passing run didn't have", () => {
    const failing = parseLog(["npm test", "Done in 12s", "", "FAIL src/login.test.ts", "Tests: 1 failed"].join("\n")).lines;
    const passing = parseLog(["npm test", "Done in 9s", "", "Tests: 42 passed"].join("\n")).lines;
    expect([...newLines(failing, passing)]).toEqual([3, 4]);
    expect(contextRows(failing, (_, i) => i === 3, 1)).toEqual([2, 3, 4]);
  });

  it("leaves lines before a truncated pass's log unmarked", () => {
    const failing = parseLog(["checkout", "install", "npm test", "FAIL x"].join("\n")).lines;
    const passingTail = parseLog(["npm test", "Tests: 42 passed"].join("\n")).lines;
    expect([...newLines(failing, passingTail, true)]).toEqual([3]);
    expect([...newLines(failing, passingTail, false)]).toEqual([0, 1, 3]);
  });

  it("formats build time", () => {
    expect(formatSecs(90)).toBe("2m");
    expect(formatSecs(7260)).toBe("2h 1m");
  });
});
