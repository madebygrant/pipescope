import type { Run } from "./types";
import { repoId } from "./types";

export type StateFilter = "all" | "failing" | "running";

export interface Pending {
  kind: "rerun" | "cancel" | "review";
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
  if (p.kind === "review") {
    const current = runs.find((r) => r.key === p.run.key);
    return !current || current.status !== "waiting";
  }
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

export function clock(ms: number) {
  const s = Math.max(0, Math.round(ms / 1000));
  const mm = Math.floor(s / 60) % 60;
  const ss = String(s % 60).padStart(2, "0");
  return s >= 3600 ? `${Math.floor(s / 3600)}:${String(mm).padStart(2, "0")}:${ss}` : `${mm}:${ss}`;
}

// Branch is left out of the key so a new branch still gets an estimate from main's runs.
const typicalKey = (r: Run) => `${r.provider}|${r.repo}|${r.title}`;
const SAMPLES = 10;

// `runs` must be newest first so the samples are the latest passes.
export function typicalDurations(runs: Run[]): Map<string, number> {
  const samples = new Map<string, number[]>();
  for (const r of runs) {
    if (r.status !== "success") continue;
    const ms = Date.parse(r.updatedAt) - Date.parse(r.startedAt ?? r.createdAt);
    const list = samples.get(typicalKey(r)) ?? [];
    if (ms > 0 && list.length < SAMPLES) samples.set(typicalKey(r), [...list, ms]);
  }
  const out = new Map<string, number>();
  for (const [key, list] of samples) {
    const sorted = list.sort((a, b) => a - b);
    const mid = sorted.length >> 1;
    out.set(key, sorted.length % 2 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2);
  }
  return out;
}

export interface Progress {
  elapsed: number;
  typical: number | null;
  // 0–1; null when there's no estimate or the run is still queued.
  fraction: number | null;
}

export function progress(run: Run, now: number, typical: Map<string, number>): Progress {
  const elapsed = now - Date.parse(run.startedAt ?? run.createdAt);
  const t = typical.get(typicalKey(run)) ?? null;
  return { elapsed, typical: t, fraction: run.status === "running" && t ? Math.min(1, elapsed / t) : null };
}

// Bitbucket runs can have no commit message.
export const headline = (r: Run) => r.subtitle ?? r.title;

export interface FailureSummary {
  // Log line number of the "Expected" line.
  n: number;
  test: string | null;
  expected: string;
  received: string;
  location: string | null;
}

// Jest qualifies some labels: "Expected length:", "Received string:", "Expected pattern:".
const EXPECTED = /^\s*Expected(?: \w+){0,2}:\s+(.+?)\s*$/;
const RECEIVED = /^\s*Received(?: \w+){0,2}:\s+(.+?)\s*$/;
// Jest's "● suite › test" and Vitest's "FAIL  file > suite > test".
const TEST_NAME = /^\s*(?:●\s+(.+)|(?:FAIL|×|✕)\s+\S+\s+>\s+(.+))$/;
// Node prints "at fn (file:line:col)"; Vitest prints "❯ file:line:col".
const FRAME = /(?:\bat\b|❯).*?\(?((?:[\w.@-]+\/)*[\w.@-]+\.\w+:\d+)(?::\d+)?\)?\s*$/;

export function failureSummary(lines: LogLine[]): FailureSummary | null {
  const at = lines.findIndex((l) => EXPECTED.test(l.text));
  if (at < 0) return null;
  const rec = lines.slice(at + 1, at + 6).find((l) => RECEIVED.test(l.text));
  if (!rec) return null;
  let test: string | null = null;
  for (let i = at - 1; i >= Math.max(0, at - 15) && !test; i--) {
    const m = lines[i].text.match(TEST_NAME);
    if (m) test = (m[1] ?? m[2]).replace(/\s+>\s+/g, " › ");
  }
  const frame = lines
    .slice(at + 1, at + 20)
    .map((l) => (l.text.includes("node_modules") ? null : l.text.match(FRAME)))
    .find(Boolean);
  return {
    n: lines[at].n,
    test,
    expected: lines[at].text.match(EXPECTED)![1],
    received: rec.text.match(RECEIVED)![1],
    location: frame ? frame[1] : null,
  };
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
// Summary lines like "0 failed" or "errors: 0" report success, so they don't count.
const ZERO_COUNT = /\b0 (errors?|failed|failures)\b|\b(errors?|failed|failures):? 0\b/gi;

export const isErrorLine = (line: string) => ERROR_LINE.test(line.replace(ZERO_COUNT, ""));

export type LineKind = "plain" | "error" | "warning" | "command" | "debug";

export interface LogLine {
  n: number;
  text: string;
  kind: LineKind;
  group: number | null;
}

export interface LogGroup {
  id: number;
  title: string;
  n: number;
  size: number;
  hasError: boolean;
}

export interface ParsedLog {
  lines: LogLine[];
  groups: LogGroup[];
}

const MARKER = /^##\[(\w+)\](.*)$/;

// Turns GitHub workflow markers (##[group], ##[error], [command]) and Bitbucket's
// "+ cmd" echo into line kinds and collapsible groups.
export function parseLog(text: string): ParsedLog {
  const lines: LogLine[] = [];
  const groups: LogGroup[] = [];
  let open: LogGroup | null = null;
  text.split("\n").forEach((raw, i) => {
    const n = i + 1;
    const marker = raw.match(MARKER);
    const tag = marker?.[1].toLowerCase();
    if (tag === "group") {
      open = { id: groups.length, title: marker![2] || "Group", n, size: 0, hasError: false };
      groups.push(open);
      return;
    }
    if (tag === "endgroup") {
      open = null;
      return;
    }
    let body = marker ? marker[2] : raw;
    let kind: LineKind = "plain";
    if (tag === "error") kind = "error";
    else if (tag === "warning") kind = "warning";
    else if (tag === "debug") kind = "debug";
    else if (tag === "command" || raw.startsWith("[command]")) {
      kind = "command";
      body = body.replace(/^\[command\]/, "");
    } else if (raw.startsWith("+ ")) kind = "command";
    else if (isErrorLine(raw)) kind = "error";
    else if (/^\s*warn(ing)?\b|\bwarning:/i.test(raw) || /\bWARN\b/.test(raw)) kind = "warning";
    const g: LogGroup | null = open;
    if (g) {
      g.size++;
      if (kind === "error") g.hasError = true;
    }
    lines.push({ n, text: body, kind, group: g ? g.id : null });
  });
  return { lines, groups };
}

// Indices into `lines` for a filtered view: each line `want` picks with `context` lines
// either side, and null where lines were skipped.
export function contextRows(lines: LogLine[], want: (l: LogLine, i: number) => boolean, context = 2): (number | null)[] {
  const keep = new Set<number>();
  lines.forEach((l, i) => {
    if (!want(l, i)) return;
    for (let j = Math.max(0, i - context); j <= Math.min(lines.length - 1, i + context); j++) keep.add(j);
  });
  const out: (number | null)[] = [];
  let last = -1;
  for (const i of [...keep].sort((a, b) => a - b)) {
    if (last >= 0 && i > last + 1) out.push(null);
    out.push(i);
    last = i;
  }
  return out;
}

export const errorContext = (lines: LogLine[], context = 2) => contextRows(lines, (l) => l.kind === "error", context);

// Hides what differs between otherwise identical lines in two runs: hashes, ids, timings, versions.
export function normaliseLine(line: string) {
  return line
    .replace(/\b[0-9a-f]{7,64}\b/gi, "#")
    .replace(/\d+/g, "#")
    .replace(/\s+/g, " ")
    .trim();
}

// Indices of lines in `failing` with no counterpart in `passing`. Blank lines never count.
// When `passing` is only the tail of its log, lines before the first shared line can't be
// compared, so they're left unmarked rather than reported as new.
export function newLines(failing: LogLine[], passing: LogLine[], passingTruncated = false): Set<number> {
  const seen = new Set(passing.map((l) => normaliseLine(l.text)));
  const norms = failing.map((l) => normaliseLine(l.text));
  const firstShared = passingTruncated ? Math.max(0, norms.findIndex((n) => n && seen.has(n))) : 0;
  const out = new Set<number>();
  norms.forEach((norm, i) => {
    if (i >= firstShared && norm && !seen.has(norm)) out.add(i);
  });
  return out;
}

export function formatSecs(secs: number) {
  const m = Math.round(secs / 60);
  if (m < 60) return `${m}m`;
  return `${Math.floor(m / 60)}h ${m % 60}m`;
}
