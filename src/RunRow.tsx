import { forwardRef } from "react";
import type { PendingDeployment, Run } from "./types";
import type { Pending } from "./model";
import { ago, duration, isActive } from "./model";
import { IconEye, StatusIcon } from "./icons";

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

export const RunRow = forwardRef<HTMLDivElement, Props>(function RunRow(p, ref) {
  const { run, now } = p;
  const active = isActive(run);
  const waiting = run.status === "waiting";
  const finished = !active && !waiting;
  const time = duration(run, now);
  const status = active && time ? `${STATUS_LABEL[run.status]} ${time}` : STATUS_LABEL[run.status];
  // A merge or bot can trigger a run of your commit; say whose commit it is.
  const by =
    run.actor && run.author && !run.author.toLowerCase().includes(run.actor.toLowerCase())
      ? `${run.actor}, commit by ${run.author}`
      : run.actor;
  const meta = [run.branch, by, ago(run.createdAt, now), finished ? time : null].filter(Boolean).join(" · ");
  const approvable = p.deployments?.filter((d) => d.canApprove) ?? [];
  const reviewers = [...new Set(p.deployments?.flatMap((d) => d.reviewers) ?? [])];

  return (
    <div
      ref={ref}
      className={`run${p.selected ? " selected" : ""}`}
      role="option"
      aria-selected={p.selected}
      onMouseEnter={p.onSelect}
    >
      <StatusIcon status={run.status} />
      <button className="run-main" onClick={p.onOpen} title="Open in browser (↵)">
        <span className="run-title">
          {p.showRepo && <span className="run-repo">{run.repo} </span>}
          {run.title} <span className="num">#{run.number}</span>
          {run.watched && (
            <span className="badge watched" title="You'll be notified when this finishes">
              <IconEye />
            </span>
          )}
        </span>
        {run.subtitle && <span className="run-sub">{run.subtitle}</span>}
        <span className="run-meta">
          {p.pending ? (
            <span className="pending">
              <span className="mini-spin" />
              {{ rerun: "Re-run requested…", cancel: "Cancelling…", review: "Sending review…" }[p.pending.kind]}
            </span>
          ) : (
            <>
              <span className={`status-text ${run.status}`}>{status}</span>
              {run.flaky && (
                <span className="badge flaky" title="Passed after failing on the same commit">
                  passed on retry
                </span>
              )}
              {run.flakyWorkflow && (
                <span className="badge flaky" title="This workflow has recently passed on retry, so a re-run may fix it (⌘R)">
                  often flaky
                </span>
              )}
              {waiting && p.deployments?.length
                ? ` · ${p.deployments.map((d) => d.environment).join(", ")} needs review${
                    approvable.length ? "" : reviewers.length ? ` by ${reviewers.join(", ")}` : ""
                  }`
                : ""}
              {meta && ` · ${meta}`}
            </>
          )}
        </span>
      </button>

      {p.confirming ? (
        <div className="actions confirm">
          <span>{CONFIRM_TEXT[p.confirming][0]}</span>
          <button className={p.confirming === "approve" ? "primary" : "danger"} onClick={p.onConfirm}>
            {CONFIRM_TEXT[p.confirming][1]}
          </button>
          <button onClick={p.onDismiss}>Back</button>
        </div>
      ) : (
        !p.pending && (
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
            <button onClick={p.onLog} title="Show log (⌘L)">
              Log
            </button>
            {active && (
              <button onClick={() => p.onAsk("cancel")} title="Cancel (⌘.)">
                Cancel
              </button>
            )}
            {run.status === "failed" && run.provider === "github" && (
              <button onClick={() => p.onRerun(true)} title="Re-run failed jobs (⌘R)">
                Re-run failed
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
                Re-run
              </button>
            )}
          </div>
        )
      )}
    </div>
  );
});
