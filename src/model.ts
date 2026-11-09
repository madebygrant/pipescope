import type { Run } from "./types";
import { repoId } from "./types";

export type StateFilter = "all" | "failing" | "running";

export interface Pending {
  kind: "rerun" | "cancel";
  at: number;
  run: Run;
}

export interface RepoGroup {
  id: string;
  repo: string;
  provider: Run["provider"];
  runs: Run[];
  pinned: boolean;
  muted: boolean;
}

export interface RepoFlags {
  pinned: Set<string>;
  muted: Set<string>;
  showMuted: boolean;
}

export interface Sections {
  failing: Run[];
  running: Run[];
  groups: RepoGroup[];
  passing: number;
  mutedRepos: number;
}

// Matches the tray icon: a failure older than this no longer counts as needing attention.
const ATTENTION_WINDOW_MS = 48 * 3600 * 1000;
const RUNS_PER_REPO = 4;

export const isActive = (r: Run) => r.status === "queued" || r.status === "running";
const workflowKey = (r: Run) => `${r.provider}|${r.repo}|${r.title}|${r.branch ?? ""}`;

export function matches(run: Run, query: string) {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  return [run.repo, run.branch, run.title, run.actor, run.subtitle].some((v) => v?.toLowerCase().includes(q));
}

// `runs` must be newest first, as the backend returns them.
export function buildSections(
  runs: Run[],
  mineOnly: boolean,
  query: string,
  now: number,
  flags: RepoFlags,
): Sections {
  const matching = runs.filter((r) => (!mineOnly || r.mine) && matches(r, query));
  const mutedRepos = new Set(matching.filter((r) => flags.muted.has(repoId(r))).map(repoId)).size;
  const visible = flags.showMuted ? matching : matching.filter((r) => !flags.muted.has(repoId(r)));
  const latest = new Map<string, Run>();
  for (const r of visible) if (!latest.has(workflowKey(r))) latest.set(workflowKey(r), r);
  const latestRuns = [...latest.values()];

  const failing = latestRuns.filter(
    (r) => r.status === "failed" && now - Date.parse(r.createdAt) < ATTENTION_WINDOW_MS,
  );
  const running = visible.filter(isActive);
  const shown = new Set([...failing, ...running].map((r) => r.key));

  const byRepo = new Map<string, RepoGroup>();
  for (const r of visible) {
    if (shown.has(r.key)) continue;
    const id = repoId(r);
    const group = byRepo.get(id) ?? {
      id,
      repo: r.repo,
      provider: r.provider,
      runs: [],
      pinned: flags.pinned.has(id),
      muted: flags.muted.has(id),
    };
    if (group.runs.length < RUNS_PER_REPO) group.runs.push(r);
    byRepo.set(id, group);
  }
  const groups = [...byRepo.values()].sort(
    (a, b) => Number(b.pinned) - Number(a.pinned) || b.runs[0].createdAt.localeCompare(a.runs[0].createdAt),
  );
  const passing = latestRuns.filter((r) => r.status === "success").length;
  return { failing, running, groups, passing, mutedRepos };
}

export function visibleOrder(s: Sections, filter: StateFilter): Run[] {
  if (filter === "failing") return s.failing;
  if (filter === "running") return s.running;
  return [...s.failing, ...s.running, ...s.groups.flatMap((g) => g.runs)];
}

export function settled(p: Pending, runs: Run[], now: number) {
  if (now - p.at > (p.kind === "rerun" ? 120_000 : 60_000)) return true;
  if (p.kind === "cancel") {
    const current = runs.find((r) => r.key === p.run.key);
    return !current || !isActive(current);
  }
  // GitHub re-runs keep the run id and go active again; Bitbucket starts a new pipeline.
  return runs.some(
    (r) =>
      workflowKey(r) === workflowKey(p.run) &&
      (r.key === p.run.key ? isActive(r) : Date.parse(r.createdAt) > Date.parse(p.run.createdAt)),
  );
}

export function elapsed(ms: number) {
  const s = Math.max(0, Math.round(ms / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${s % 60}s`;
  return `${Math.floor(m / 60)}h ${m % 60}m`;
}

export function duration(run: Run, now: number): string | null {
  const start = Date.parse(run.startedAt ?? run.createdAt);
  if (isActive(run)) return elapsed(now - start);
  if (run.status === "success" || run.status === "failed" || run.status === "cancelled") {
    return elapsed(Date.parse(run.updatedAt) - start);
  }
  return null;
}

export function ago(iso: string, now: number) {
  const secs = Math.max(0, (now - Date.parse(iso)) / 1000);
  if (secs < 60) return "just now";
  if (secs < 3600) return `${Math.floor(secs / 60)}m ago`;
  if (secs < 86400) return `${Math.floor(secs / 3600)}h ago`;
  return `${Math.floor(secs / 86400)}d ago`;
}

// Matches lines worth jumping to in a CI log.
const ERROR_LINE = /##\[error\]|\berror\b|\bfailed\b|\bfailure\b|exception|panicked|traceback|\bFAIL\b|ERR!/i;

export const isErrorLine = (line: string) => ERROR_LINE.test(line);
