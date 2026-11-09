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
    ...extra,
  };
}

const runs: Run[] = [
  run("hatchd/web", "CI", "running", 2, { branch: "feature/cart" }),
  run("hatchd/api", "Deploy", "queued", 1),
  run("hatchd/web", "CI", "failed", 20, { branch: "feature/login" }),
  run("hatchd/web", "CI", "success", 40),
  run("hatchd/api", "Tests", "failed", 55, { branch: "fix/timeouts", subtitle: "Raise the DB pool size" }),
  run("hatchd/api", "Tests", "success", 90),
  run("hatchd/mobile", "Build", "success", 120, { provider: "bitbucket", key: "bitbucket:hatchd/mobile:{x}", subtitle: null }),
  run("hatchd/mobile", "Build", "cancelled", 300, { provider: "bitbucket", key: "bitbucket:hatchd/mobile:{y}", subtitle: null }),
  run("hatchd/docs", "Pages", "waiting", 30, { actor: "sam", mine: false }),
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
    state: "notConfigured",
    account: null,
    message: null,
    repos: 0,
    source: null,
    lastPoll: null,
    rate: null,
    warnings: [],
  },
];

const settings = { notifyAll: false, bitbucketEmail: null, pinned: ["github:hatchd/api"], muted: ["github:hatchd/docs"], onboarded: true };
const log = [
  "Run npm test",
  "> web@1.0.0 test",
  "PASS src/cart.test.ts",
  "FAIL src/login.test.ts",
  "  ● login › rejects a bad password",
  "    expect(received).toBe(expected)",
  "    Expected: 401",
  "    Received: 500",
  ...Array.from({ length: 30 }, (_, i) => `    at step ${i}`),
  "Tests: 1 failed, 41 passed, 42 total",
  "##[error]Process completed with exit code 1.",
].join("\n");

const responses: Record<string, unknown> = {
  get_runs: runs,
  get_status: statuses,
  get_settings: settings,
  get_log: {
    stepId: "2",
    step: "test",
    steps: [
      { id: "1", name: "build", status: "success" },
      { id: "2", name: "test", status: "failed" },
    ],
    text: log,
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
      return responses[cmd] ?? null;
    },
  };
  (window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
}
