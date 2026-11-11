import { describe, expect, it } from "vitest";
import { failureMarkdown } from "./agentReport";
import type { LogTail, Run } from "./types";

const run = (extra: Partial<Run> = {}): Run => ({
  key: "github:acme/web:7",
  provider: "github",
  repo: "acme/web",
  id: "7",
  number: 103,
  title: "CI",
  subtitle: "Tidy up the login form",
  branch: "feature/login",
  commit: "abcdef1234567890",
  status: "failed",
  actor: "grant",
  mine: true,
  createdAt: "2026-11-11T00:00:00Z",
  startedAt: null,
  updatedAt: "2026-11-11T00:03:00Z",
  webUrl: "https://github.com/acme/web/actions/runs/7",
  author: "Grant",
  buildSecs: null,
  flaky: false,
  flakyWorkflow: false,
  watched: false,
  ...extra,
});

const log = (text: string, extra: Partial<LogTail> = {}): LogTail => ({
  stepId: "2",
  step: "test",
  steps: [
    { id: "1", name: "lint", status: "failed" },
    { id: "2", name: "test", status: "failed" },
  ],
  text,
  skipped: 0,
  webUrl: "https://github.com/acme/web/actions/runs/7/job/2",
  ...extra,
});

const failing = [
  "> web@1.0.0 test",
  "PASS src/cart.test.ts",
  "FAIL src/login.test.ts",
  "  ● login › rejects a bad password",
  "    Expected: 401",
  "    Received: 500",
  "      at Object.<anonymous> (src/login.test.ts:40:5)",
  "Tests: 1 failed, 41 passed",
].join("\n");

describe("failureMarkdown", () => {
  it("covers the run, test failure, errors, new lines and changed files", () => {
    const base = run({ key: "b", number: 99, branch: "main", commit: "1234567abcdef", status: "success" });
    const md = failureMarkdown({
      run: run(),
      log: log(failing),
      baseline: { run: base, log: log("> web@1.0.0 test\nPASS src/cart.test.ts\nTests: 42 passed") },
      changes: { base, files: [{ path: "src/login.ts", status: "modified" }] },
      checkout: "/Users/me/Projects/web",
    });
    expect(md).toContain("# CI failure: acme/web · CI #103");
    expect(md).toContain("- Commit: `abcdef1` Tidy up the login form (Grant)");
    expect(md).toContain("- Other failed steps: lint");
    expect(md).toContain("- Local checkout: `/Users/me/Projects/web`");
    expect(md).toContain("`login › rejects a bad password` at `src/login.test.ts:40`");
    expect(md).toContain("- Expected: `401`\n- Received: `500`");
    expect(md).toContain("FAIL src/login.test.ts");
    expect(md).toContain("weren't in the last pass, #99 (`1234567` on `main`)");
    const newSection = md.slice(md.indexOf("## New since"), md.indexOf("## Changed files"));
    expect(newSection).toContain("FAIL src/login.test.ts");
    expect(newSection).not.toContain("PASS src/cart.test.ts");
    expect(md).toContain("Since the last pass at `1234567` (#99):\n\n- modified `src/login.ts`");
  });

  it("says when the last pass ran the same commit, and skips the file list", () => {
    const base = run({ key: "b", number: 99, status: "success" });
    const md = failureMarkdown({
      run: run({ flakyWorkflow: true }),
      log: log(failing),
      baseline: null,
      changes: { base, files: [] },
      checkout: null,
    });
    expect(md).toContain("may be flaky");
    expect(md).toContain("ran this same commit");
    expect(md).not.toContain("## Changed files");
    expect(md).toContain("not found in the folders Pipescope searches");
    expect(md).toContain("No earlier passing run of CI");
  });

  it("reports what couldn't be fetched and fences lines that hold backticks", () => {
    const md = failureMarkdown({
      run: run(),
      log: log("error: bad ```template```", { skipped: 900 }),
      baseline: null,
      changes: null,
      checkout: null,
      missing: { baseline: "rate limited", changes: "404" },
    });
    expect(md).toContain("From the last line of the log.");
    expect(md).toContain("````text\nerror: bad ```template```\n````");
    expect(md).toContain("Couldn't load the last passing run: rate limited");
    expect(md).toContain("Couldn't list them: 404");
  });

  it("works without a log", () => {
    const md = failureMarkdown({ run: run(), log: null, baseline: null, changes: null, checkout: null, missing: { log: "offline" } });
    expect(md).toContain("Couldn't load the log: offline");
    expect(md).not.toContain("## New since");
  });
});
