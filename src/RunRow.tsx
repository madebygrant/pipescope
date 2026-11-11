import { forwardRef } from "react";
import type { PendingDeployment, Run } from "./types";
import type { Pending, Progress } from "./model";
import { ago, clock, duration, headline, isActive } from "./model";
import { IconEye } from "./icons";

const STATUS_LABEL: Record<Run["status"], string> = {
  queued: "Queued",
  running: "Running",
  waiting: "Waiting",
  success: "Passed",
  failed: "Failed",
  cancelled: "Cancelled",
  skipped: "Skipped",
};

export type Confirm = "cancel" | "approve" | "reject";

interface Props {
  run: Run;
  now: number;
  selected: boolean;
  pending?: Pending;
  confirming: Confirm | null;
  showRepo?: boolean;
  // Only for queued and running runs.
  progress?: Progress;
  // GitHub deployments waiting on review; undefined until loaded.
  deployments?: PendingDeployment[];
  onSelect: () => void;
  onOpen: () => void;
  onLog: () => void;
  onRerun: (failedOnly: boolean) => void;
  onAsk: (action: Confirm) => void;
  onConfirm: () => void;
  onDismiss: () => void;
  onWatch: () => void;
  onOpenStep: () => void;
}

const CONFIRM_TEXT: Record<Confirm, [string, string]> = {
  cancel: ["Cancel this run?", "Cancel run"],
  approve: ["Approve the deployment?", "Approve"],
  reject: ["Reject the deployment?", "Reject"],
};

function progressLabel(run: Run, progress: Progress) {
  if (run.status === "queued") return `Queued ${clock(progress.elapsed)}`;
  return progress.typical ? `${clock(progress.elapsed)} / ~${clock(progress.typical)}` : clock(progress.elapsed);
}

function ProgressLine({ run, progress }: { run: Run; progress: Progress }) {
  const queued = run.status === "queued";
  const label = progressLabel(run, progress);
  return (
    <span className="run-progress">
      {queued ? (
        <span className="track queued" />
      ) : (
        progress.fraction !== null && (
          <span className="track">
            <span className="fill" style={{ width: `${progress.fraction * 100}%` }} />
          </span>
        )
      )}
      <span className="progress-label">{label}</span>
    </span>
  );
}

export const RunRow = forwardRef<HTMLDivElement, Props>(function RunRow(p, ref) {
  const { run, now } = p;
  const active = isActive(run);
  const waiting = run.status === "waiting";
  const finished = !active && !waiting;
  const time = duration(run, now);
  // A merge or bot can trigger a run of your commit; say whose commit it is.
  const by =
    run.actor && run.author && !run.author.toLowerCase().includes(run.actor.toLowerCase())
      ? `${run.actor}, commit by ${run.author}`
      : run.actor;
  const workflow = p.showRepo ? `${run.repo}/${run.title}` : run.subtitle ? run.title : null;
  const meta = [workflow, run.branch, by, ago(run.createdAt, now), finished ? time : null].filter(Boolean).join(" · ");
  const approvable = p.deployments?.filter((d) => d.canApprove) ?? [];
  const reviewers = [...new Set(p.deployments?.flatMap((d) => d.reviewers) ?? [])];

  const flags = [
    run.flakyWorkflow ? "often flaky" : null,
    waiting && p.deployments?.length
      ? `${p.deployments.map((d) => d.environment).join(", ")} needs review${
          approvable.length ? "" : reviewers.length ? ` by ${reviewers.join(", ")}` : ""
        }`
      : null,
  ].filter(Boolean);
  const label = run.status === "success" && run.flaky ? "Passed on retry" : STATUS_LABEL[run.status];
  const state = [label, ...flags].join(" · ");
  // The selected row swaps its status line for buttons, so the status moves into the meta line.
  const showActions = (p.selected || !!p.confirming) && !p.pending;
  const stateText = active && p.progress ? progressLabel(run, p.progress) : state;

  return (
    <div
      ref={ref}
      className={`run ${run.status}${p.selected ? " selected" : ""}`}
      role="option"
      aria-selected={p.selected}
      onMouseEnter={p.onSelect}
      onFocus={p.onSelect}
    >
      <span className={`run-num${String(run.number).length > 4 ? " long" : ""}`} aria-hidden>
        {run.number}
      </span>
      <div className="run-body">
        <button className="run-main" onClick={p.onOpen} title="Open in browser (↵)">
          <span className="run-title">
            {headline(run)}
            <span className="visually-hidden"> #{run.number}</span>
            {run.watched && (
              <span className="badge watched" title="You'll be notified when this finishes">
                <IconEye />
              </span>
            )}
          </span>
          <span className="run-meta">
            {showActions && <span className={`run-state ${run.status}`}>{stateText} · </span>}
            {meta}
          </span>
        </button>
        {!showActions && (p.pending ? (
          <span className="run-state pending">
            <span className="mini-spin" />
            {{ rerun: "Re-run requested…", cancel: "Cancelling…", review: "Sending review…" }[p.pending.kind]}
          </span>
        ) : active && p.progress ? (
          <ProgressLine run={run} progress={p.progress} />
        ) : (
          <span className={`run-state ${run.status}`} title={run.flakyWorkflow ? "This workflow has recently passed on retry, so a re-run may fix it (⌘R)" : undefined}>
            {state}
          </span>
        ))}

        {p.confirming ? (
          <div className="actions confirm">
            <span>{CONFIRM_TEXT[p.confirming][0]}</span>
            <button className={p.confirming === "approve" ? "primary" : "danger"} onClick={p.onConfirm}>
              {CONFIRM_TEXT[p.confirming][1]}
            </button>
            <button onClick={p.onDismiss}>Back</button>
          </div>
        ) : (
          showActions && (
            <div className="actions">
              {approvable.length > 0 && (
                <>
                  <button className="primary" onClick={() => p.onAsk("approve")} title="Approve the deployment">
                    Approve
                  </button>
                  <button onClick={() => p.onAsk("reject")} title="Reject the deployment">
                    Reject
                  </button>
                </>
              )}
              {waiting && run.provider === "bitbucket" && (
                <button onClick={p.onOpenStep} title="Bitbucket can only run manual steps from its website">
                  Open step
                </button>
              )}
              {!finished && (
                <button onClick={p.onWatch} title={run.watched ? "Stop watching (⌘D)" : "Notify me when this finishes (⌘D)"}>
                  {run.watched ? "Unwatch" : "Watch"}
                </button>
              )}
              {run.status === "failed" && run.provider === "github" && (
                <button className="primary" onClick={() => p.onRerun(true)} title="Re-run failed jobs (⌘R)">
                  Re-run failed
                </button>
              )}
              {active && (
                <button onClick={() => p.onAsk("cancel")} title="Cancel (⌘.)">
                  Cancel
                </button>
              )}
              {finished && (
                <button
                  onClick={() => p.onRerun(false)}
                  title={
                    run.provider === "bitbucket"
                      ? "Start a new pipeline on the same commit (⌘R)"
                      : run.status === "failed"
                        ? "Re-run all jobs (⇧⌘R)"
                        : "Re-run all jobs (⌘R)"
                  }
                >
                  {run.status === "failed" && run.provider === "github" ? "Re-run all" : "Re-run"}
                </button>
              )}
              <button className="open-log" onClick={p.onLog} title="Show log (⌘L)">
                Log
              </button>
            </div>
          )
        )}
      </div>
    </div>
  );
});
