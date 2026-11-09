import { forwardRef } from "react";
import type { Run } from "./types";
import type { Pending } from "./model";
import { ago, duration, isActive } from "./model";
import { StatusIcon } from "./icons";

const STATUS_LABEL: Record<Run["status"], string> = {
  queued: "Queued",
  running: "Running",
  waiting: "Waiting",
  success: "Passed",
  failed: "Failed",
  cancelled: "Cancelled",
  skipped: "Skipped",
};

interface Props {
  run: Run;
  now: number;
  selected: boolean;
  pending?: Pending;
  confirming: boolean;
  showRepo?: boolean;
  onSelect: () => void;
  onOpen: () => void;
  onLog: () => void;
  onRerun: (failedOnly: boolean) => void;
  onCancel: () => void;
  onConfirmCancel: () => void;
  onKeepRunning: () => void;
}

export const RunRow = forwardRef<HTMLDivElement, Props>(function RunRow(p, ref) {
  const { run, now } = p;
  const active = isActive(run);
  const finished = !active && run.status !== "waiting";
  const time = duration(run, now);
  const status = active && time ? `${STATUS_LABEL[run.status]} ${time}` : STATUS_LABEL[run.status];
  const meta = [run.branch, run.actor, ago(run.createdAt, now), finished ? time : null].filter(Boolean).join(" · ");

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
        </span>
        {run.subtitle && <span className="run-sub">{run.subtitle}</span>}
        <span className="run-meta">
          {p.pending ? (
            <span className="pending">
              <span className="mini-spin" />
              {p.pending.kind === "rerun" ? "Re-run requested…" : "Cancelling…"}
            </span>
          ) : (
            <>
              <span className={`status-text ${run.status}`}>{status}</span>
              {meta && ` · ${meta}`}
            </>
          )}
        </span>
      </button>

      {p.confirming ? (
        <div className="actions confirm">
          <span>Cancel this run?</span>
          <button className="danger" onClick={p.onConfirmCancel}>
            Cancel run
          </button>
          <button onClick={p.onKeepRunning}>Keep</button>
        </div>
      ) : (
        !p.pending && (
          <div className="actions">
            <button onClick={p.onLog} title="Show log (⌘L)">
              Log
            </button>
            {active && (
              <button onClick={p.onCancel} title="Cancel (⌘.)">
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
