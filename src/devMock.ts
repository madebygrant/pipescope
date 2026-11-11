// Fake Tauri backend so the UI can be checked in a plain browser with `pnpm dev`.
import type { ProviderStatus, Run, RunStatus } from "./types";

const min = 60_000;
let id = 0;
function run(repo: string, title: string, status: RunStatus, agoMin: number, extra: Partial<Run> = {}): Run {
  const created = new Date(Date.now() - agoMin * min).toISOString();
  const done = status === "running" || status === "queued" ? created : new Date(Date.now() - (agoMin - 3) * min).toISOString();
  id += 1;
  return {
    key: `github:${repo}:${id}`,
    provider: "github",
    repo,
    id: String(id),
    number: 100 + id,
    title,
    subtitle: "Tidy up the checkout flow",
    branch: "main",
    commit: "abc1234",
    status,
    actor: "grant",
    mine: true,
    createdAt: created,
    startedAt: created,
    updatedAt: done,
    webUrl: "https://example.com",
    author: "Grant",
    buildSecs: status === "running" || status === "queued" ? null : 180,
    flaky: false,
    flakyWorkflow: false,
    watched: false,
    ...extra,
  };
}

const runs: Run[] = [
  run("acme/web", "CI", "running", 2, { branch: "feature/cart", subtitle: "Add saved carts" }),
  run("acme/api", "Deploy", "queued", 1, { subtitle: "Release 2.14" }),
  run("acme/web", "CI", "failed", 20, { branch: "feature/login", flakyWorkflow: true, subtitle: "Tidy up the login form" }),
  run("acme/web", "CI", "success", 40, { subtitle: "Bump dependencies" }),
  run("acme/api", "Tests", "failed", 55, { branch: "fix/timeouts", subtitle: "Raise the DB pool size" }),
  run("acme/api", "Tests", "success", 90, { flaky: true, subtitle: "Cache rate lookups" }),
  run("globex/mobile", "Build", "success", 120, { provider: "bitbucket", key: "bitbucket:globex/mobile:{x}", subtitle: null }),
  run("globex/mobile", "Build", "cancelled", 300, { provider: "bitbucket", key: "bitbucket:globex/mobile:{y}", subtitle: null }),
  run("acme/docs", "Pages", "waiting", 30, { actor: "sam", mine: false, title: "Deploy" }),
  run("acme/web", "CI", "running", 4, { actor: "merge-bot", author: "Grant", branch: "main", subtitle: "Merge pull request #212" }),
].sort((a, b) => b.createdAt.localeCompare(a.createdAt));

const statuses: ProviderStatus[] = [
  {
    provider: "github",
    state: "connected",
    account: "grant",
    message: null,
    repos: 41,
    source: "gh CLI",
    lastPoll: new Date(Date.now() - 8000).toISOString(),
    rate: { remaining: 4210, limit: 5000, resetSecs: 1800 },
    warnings: [],
  },
  {
    provider: "bitbucket",
    state: "connected",
    account: "Grant",
    message: null,
    repos: 12,
    source: "Keychain",
    lastPoll: new Date(Date.now() - 12000).toISOString(),
    rate: null,
    warnings: [],
  },
];

const settings = {
  notifyAll: false,
  bitbucketEmail: null,
  pinned: ["github:acme/api"],
  muted: ["github:acme/docs"],
  onboarded: true,
  launchAtLogin: true,
  notifyBranches: "",
  quietHours: "",
  autoRerunFlaky: false,
  clients: { "github:acme/web": "Acme", "github:acme/api": "Acme", "bitbucket:globex/mobile": "Globex" } as Record<string, string>,
  codeFolders: "",
  defaultCodeFolders: "~/Projects\n~/Developer\n~/Code",
};
const log = [
  "##[group]Run actions/checkout@v4",
  "with:",
  "  repository: acme/web",
  "##[endgroup]",
  "Syncing repository: acme/web",
  "[command]/usr/bin/git version",
  "git version 2.47.1",
  "##[group]Run npm ci",
  "npm ci",
  "##[endgroup]",
  "npm WARN deprecated inflight@1.0.6: This module is not supported",
  "added 812 packages in 14s",
  "##[group]Run npm test",
  "npm test",
  "##[endgroup]",
  "> web@1.0.0 test",
  "PASS src/cart.test.ts",
  "FAIL src/login.test.ts",
  "  ● login › rejects a bad password",
  "    expect(received).toBe(expected)",
  "    Expected: 401",
  "    Received: 500",
  ...Array.from({ length: 5 }, (_, i) => `      at Object.<anonymous> (src/login.test.ts:${40 + i}:5)`),
  "Test Suites: 1 failed, 11 passed, 12 total",
  "Tests:       1 failed, 41 passed, 42 total",
  "Snapshots:   0 total",
  "##[error]Process completed with exit code 1.",
].join("\n");

const responses: Record<string, unknown> = {
  get_runs: runs,
  get_status: statuses,
  get_settings: settings,
  get_pending_deployments: [{ environmentId: 1, environment: "production", canApprove: true, reviewers: ["grant", "sam"] }],
  manual_step_url: "https://example.com/steps/1",
  get_build_time: [
    { repoId: "github:acme/web", secs: 7260, runs: 31 },
    { repoId: "github:acme/api", secs: 3120, runs: 14 },
    { repoId: "bitbucket:globex/mobile", secs: 5400, runs: 9 },
  ],
  get_baseline_log: {
    run: { ...runs[runs.length - 1], number: 99, branch: "main", status: "success" },
    log: { stepId: "2", step: "test", steps: [], skipped: 0, webUrl: "https://example.com",
      text: log.split("\n").filter((l) => !/FAIL|login|Expected|Received|anonymous|failed|error/i.test(l)).join("\n") + "\nTests: 42 passed, 42 total" },
  },
  get_checkouts: { "github:acme/web": "/Users/me/Projects/acme/web", "github:acme/api": "/Users/me/Projects/acme/api" },
  get_changed_files: {
    base: { ...runs[runs.length - 1], number: 99, branch: "main", status: "success", commit: "9f8e7d6c5b4a" },
    files: [
      { path: "src/login.ts", status: "modified" },
      { path: "src/session.ts", status: "added" },
    ],
  },
  get_log: {
    stepId: "2",
    step: "test",
    steps: [
      { id: "1", name: "build", status: "success" },
      { id: "2", name: "test", status: "failed" },
    ],
    text: log,
    skipped: 1240,
    webUrl: "https://example.com",
  },
};

export function install() {
  let callbackId = 0;
  (window as any).__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: "main" }, currentWebview: { windowLabel: "main", label: "main" } },
    transformCallback: () => ++callbackId,
    unregisterCallback: () => {},
    invoke: async (cmd: string, args: Record<string, unknown>) => {
      if (cmd === "rerun" || cmd === "cancel") await new Promise((r) => setTimeout(r, 400));
      if (cmd === "set_repo_flag") {
        const list = settings[args.flag as "pinned" | "muted"];
        const id = args.repoId as string;
        const at = list.indexOf(id);
        if (args.on && at < 0) list.push(id);
        if (!args.on && at >= 0) list.splice(at, 1);
      }
      // Fresh copies, as a real backend would send, so React sees each change.
      if (cmd === "get_runs") return runs.map((r) => ({ ...r }));
      if (cmd === "get_settings") return { ...settings, clients: { ...settings.clients } };
      if (cmd === "get_run") return runs.find((r) => r.key === args.key) ?? null;
      if (cmd === "watch_run") {
        const r = runs.find((x) => x.key === args.key);
        if (r) r.watched = args.on as boolean;
      }
      if (cmd === "set_repo_client") {
        if (args.client) settings.clients[args.repoId as string] = args.client as string;
        else delete settings.clients[args.repoId as string];
      }
      return responses[cmd] ?? null;
    },
  };
  (window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
}
