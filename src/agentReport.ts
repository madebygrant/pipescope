import { api, errorText, loadBaseline } from "./api";
import type { Baseline, Changes, LogTail, Run } from "./types";
import { repoId } from "./types";
import { contextRows, failureSummary, headline, newLines, parseLog } from "./model";

// Enough to point an agent at the problem without filling its context with the whole log.
const MAX_ERROR_ROWS = 120;
const MAX_NEW_LINES = 80;
const MAX_FILES = 100;
const TAIL_WITHOUT_ERRORS = 30;

export interface ReportInput {
  run: Run;
  log: LogTail | null;
  baseline: Baseline | null;
  changes: Changes | null;
  checkout: string | null;
  // Why a part couldn't be fetched.
  missing?: Partial<Record<"log" | "baseline" | "changes", string>>;
}

const short = (sha: string | null | undefined) => (sha ? sha.slice(0, 7) : "unknown");

// A log line holding ``` would end a three-backtick fence early, so the fence outgrows the longest run.
function fence(lines: string[]) {
  const longest = Math.max(0, ...lines.flatMap((l) => (l.match(/`+/g) ?? []).map((s) => s.length)));
  const tick = "`".repeat(Math.max(3, longest + 1));
  return [tick + "text", ...lines, tick].join("\n");
}

function rowsBlock(rows: (number | null)[], text: (i: number) => string, max: number) {
  const shown = rows.slice(0, max).map((i) => (i === null ? "…" : text(i)));
  const rest = rows.slice(max).filter((i) => i !== null).length;
  return fence(shown) + (rest ? `\n\n${rest} more lines not shown.` : "");
}

export function failureMarkdown({ run, log, baseline, changes, checkout, missing = {} }: ReportInput): string {
  const out: string[] = [`# CI failure: ${run.repo} · ${run.title} #${run.number}`, ""];
  const lines = log ? parseLog(log.text).lines : [];
  const failedSteps = log?.steps.filter((s) => s.status === "failed").map((s) => s.name) ?? [];

  out.push(`- Run: ${log?.webUrl || run.webUrl}`);
  if (run.branch) out.push(`- Branch: \`${run.branch}\``);
  if (run.commit) out.push(`- Commit: \`${short(run.commit)}\` ${headline(run)}${run.author ? ` (${run.author})` : ""}`);
  if (log?.step) out.push(`- Step: ${log.step}`);
  const others = failedSteps.filter((s) => s !== log?.step);
  if (others.length) out.push(`- Other failed steps: ${others.join(", ")}`);
  out.push(`- Local checkout: ${checkout ? `\`${checkout}\`` : "not found in the folders Pipescope searches"}`);

  const sameCommit = !!changes?.base && changes.base.commit === run.commit;
  const notes: string[] = [];
  if (run.flakyWorkflow) {
    notes.push("This workflow recently failed and then passed on a retry of the same commit, so this failure may be flaky.");
  }
  if (sameCommit) {
    notes.push(`The last pass, #${changes!.base!.number}, ran this same commit, so the code hasn't changed since it passed.`);
  }
  if (notes.length) out.push("", ...notes);

  const summary = run.status === "failed" ? failureSummary(lines) : null;
  if (summary) {
    out.push("", "## Test failure", "");
    if (summary.test || summary.location) {
      out.push([summary.test && `\`${summary.test}\``, summary.location && `at \`${summary.location}\``].filter(Boolean).join(" "), "");
    }
    out.push(`- Expected: \`${summary.expected}\``, `- Received: \`${summary.received}\``);
  }

  out.push("", "## Errors", "");
  if (!log) {
    out.push(`Couldn't load the log${missing.log ? `: ${missing.log}` : "."}`);
  } else {
    if (log.skipped) out.push(`From the last ${lines.length === 1 ? "line" : `${lines.length} lines`} of the log. The full log is at ${log.webUrl || run.webUrl}.`, "");
    const rows = contextRows(lines, (l) => l.kind === "error");
    if (rows.length) {
      out.push("Each error line with two lines either side:", "", rowsBlock(rows, (i) => lines[i].text, MAX_ERROR_ROWS));
    } else {
      out.push(`No lines look like errors. The last ${TAIL_WITHOUT_ERRORS}:`, "", fence(lines.slice(-TAIL_WITHOUT_ERRORS).map((l) => l.text)));
    }
  }

  if (log) {
    out.push("", "## New since the last pass", "");
    if (missing.baseline) {
      out.push(`Couldn't load the last passing run: ${missing.baseline}`);
    } else if (!baseline) {
      out.push(`No earlier passing run of ${run.title} to compare with.`);
    } else if (!baseline.log.stepId) {
      out.push(`The last pass, #${baseline.run.number}, has no step called ${log.step}.`);
    } else {
      const changed = newLines(lines, parseLog(baseline.log.text).lines, baseline.log.skipped > 0);
      const where = `#${baseline.run.number} (\`${short(baseline.run.commit)}\`${baseline.run.branch ? ` on \`${baseline.run.branch}\`` : ""})`;
      if (!changed.size) {
        out.push(`Every line also appears in the last pass, ${where}.`);
      } else {
        const rows = contextRows(lines, (_, i) => changed.has(i), 0);
        out.push(`${changed.size} lines weren't in the last pass, ${where}:`, "", rowsBlock(rows, (i) => lines[i].text, MAX_NEW_LINES));
      }
    }
  }

  if (!sameCommit && (changes || missing.changes)) {
    out.push("", "## Changed files", "");
    if (!changes) {
      out.push(`Couldn't list them: ${missing.changes}`);
    } else {
      const base = changes.base;
      out.push(base ? `Since the last pass at \`${short(base.commit)}\` (#${base.number}):` : `In commit \`${short(run.commit)}\`:`, "");
      if (!changes.files.length) out.push("None.");
      for (const f of changes.files.slice(0, MAX_FILES)) out.push(`- ${f.status} \`${f.path}\``);
      if (changes.files.length > MAX_FILES) out.push(`- and ${changes.files.length - MAX_FILES} more`);
    }
  }

  return out.join("\n") + "\n";
}

// `have` passes in what the log view already loaded, so it isn't fetched twice.
export async function failureReport(run: Run, have?: { log: LogTail; baseline?: Baseline | null }): Promise<string> {
  const missing: NonNullable<ReportInput["missing"]> = {};
  const why = (part: keyof typeof missing) => (e: unknown) => {
    missing[part] = errorText(e);
    return null;
  };
  const log = have?.log ?? (await api.log(run.key).catch(why("log")));
  const [baseline, changes, checkouts] = await Promise.all([
    have?.baseline !== undefined ? have.baseline : log?.stepId ? loadBaseline(run.key, log.step).catch(why("baseline")) : null,
    run.commit ? api.changedFiles(run.key).catch(why("changes")) : null,
    api.checkouts().catch(() => ({}) as Record<string, string>),
  ]);
  return failureMarkdown({ run, log, baseline, changes, checkout: checkouts[repoId(run).toLowerCase()] ?? null, missing });
}
