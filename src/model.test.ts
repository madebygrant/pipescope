import { describe, expect, it } from "vitest";
import {
  clock,
  contextRows,
  errorContext,
  failureSummary,
  formatSecs,
  isErrorLine,
  newLines,
  normaliseLine,
  parseLog,
  progress,
  typicalDurations,
} from "./model";
import type { Run } from "./types";

const at = (min: number) => new Date(Date.UTC(2026, 0, 1, 12, min)).toISOString();
function run(status: Run["status"], start: number, end: number, extra: Partial<Run> = {}): Run {
  return {
    key: `k${start}`,
    provider: "github",
    repo: "acme/web",
    id: "1",
    number: 1,
    title: "CI",
    subtitle: null,
    branch: "main",
    commit: null,
    status,
    actor: null,
    mine: true,
    createdAt: at(start),
    startedAt: at(start),
    updatedAt: at(end),
    webUrl: "",
    author: null,
    buildSecs: null,
    flaky: false,
    flakyWorkflow: false,
    watched: false,
    ...extra,
  };
}

describe("estimating how long a run takes", () => {
  it("takes the median of recent passes, across branches, ignoring failures", () => {
    const runs = [run("success", 50, 54), run("failed", 40, 41), run("success", 30, 33, { branch: "feat" }), run("success", 20, 25)];
    expect(typicalDurations(runs).get("github|acme/web|CI")).toBe(4 * 60_000);
    expect(typicalDurations([run("success", 0, 2), run("success", 10, 14)]).get("github|acme/web|CI")).toBe(3 * 60_000);
  });

  it("gives a fraction only for running runs with an estimate", () => {
    const typical = typicalDurations([run("success", 0, 4)]);
    const now = Date.parse(at(62));
    expect(progress(run("running", 60, 60), now, typical)).toEqual({ elapsed: 120_000, typical: 240_000, fraction: 0.5 });
    expect(progress(run("running", 50, 50), now, typical).fraction).toBe(1);
    expect(progress(run("queued", 61, 61), now, typical).fraction).toBeNull();
    expect(progress(run("running", 60, 60, { title: "Other" }), now, typical).fraction).toBeNull();
  });

  it("formats a timer", () => {
    expect(clock(125_000)).toBe("2:05");
    expect(clock(3_725_000)).toBe("1:02:05");
  });
});

describe("failureSummary", () => {
  it("reads Jest output", () => {
    const { lines } = parseLog(
      [
        "FAIL src/login.test.ts",
        "  ● login › rejects a bad password",
        "    expect(received).toBe(expected)",
        "    Expected: 401",
        "    Received: 500",
        "      at Object.<anonymous> (node_modules/jest/x.js:1:1)",
        "      at Object.<anonymous> (src/login.test.ts:40:5)",
      ].join("\n"),
    );
    expect(failureSummary(lines)).toEqual({
      n: 4,
      test: "login › rejects a bad password",
      expected: "401",
      received: "500",
      location: "src/login.test.ts:40",
    });
  });

  it("reads Vitest output", () => {
    const { lines } = parseLog(
      [" FAIL  src/cart.test.ts > cart > totals", "AssertionError: expected 3 to be 4", "- Expected: 4", "Expected: 4", "Received: 3", " ❯ src/cart.test.ts:12:20"].join("\n"),
    );
    expect(failureSummary(lines)).toMatchObject({
      test: "cart › totals",
      expected: "4",
      received: "3",
      location: "src/cart.test.ts:12",
    });
  });

  it("reads Jest's qualified labels", () => {
    const { lines } = parseLog(["  ● list › has three", "    Expected length: 3", "    Received length: 2"].join("\n"));
    expect(failureSummary(lines)).toMatchObject({ test: "list › has three", expected: "3", received: "2" });
  });

  it("finds nothing in other logs", () => {
    expect(failureSummary(parseLog("error: build failed\nExpected: 1").lines)).toBeNull();
  });
});

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
