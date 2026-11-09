import { useEffect, useMemo, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api, errorText } from "./api";
import type { LogTail, Run } from "./types";
import { isErrorLine } from "./model";
import { IconBack, IconCopy, IconExternal, StatusIcon } from "./icons";

export function LogView({ run, onBack }: { run: Run; onBack: () => void }) {
  const [log, setLog] = useState<LogTail | null>(null);
  const [step, setStep] = useState<string | undefined>();
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [errorIndex, setErrorIndex] = useState(0);
  const lineRefs = useRef<(HTMLDivElement | null)[]>([]);
  const bodyRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    setLog(null);
    setError(null);
    api.log(run.key, step).then(setLog).catch((e) => setError(errorText(e)));
  }, [run.key, step]);

  const lines = useMemo(() => log?.text.split("\n") ?? [], [log]);
  const errorLines = useMemo(() => lines.flatMap((l, i) => (isErrorLine(l) ? [i] : [])), [lines]);

  // Open on the first error rather than the end, where the useful line usually isn't.
  useEffect(() => {
    if (!log) return;
    setErrorIndex(0);
    requestAnimationFrame(() => {
      const first = errorLines[0];
      if (first !== undefined) lineRefs.current[first]?.scrollIntoView({ block: "center" });
      else if (bodyRef.current) bodyRef.current.scrollTop = bodyRef.current.scrollHeight;
    });
  }, [log, errorLines]);

  const jump = (delta: number) => {
    if (!errorLines.length) return;
    const next = (errorIndex + delta + errorLines.length) % errorLines.length;
    setErrorIndex(next);
    lineRefs.current[errorLines[next]]?.scrollIntoView({ block: "center" });
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey && e.key.toLowerCase() === "g") {
        e.preventDefault();
        jump(e.shiftKey ? -1 : 1);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const copy = async () => {
    if (!log) return;
    try {
      await navigator.clipboard.writeText(log.text);
    } catch {
      const area = document.createElement("textarea");
      area.value = log.text;
      document.body.appendChild(area);
      area.select();
      document.execCommand("copy");
      area.remove();
    }
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1500);
  };

  const current = errorLines[errorIndex];
  return (
    <div className="app">
      <header className="bar">
        <button className="icon" title="Back (esc)" aria-label="Back" onClick={onBack}>
          <IconBack />
        </button>
        <div className="log-title">
          <strong>
            {run.title} #{run.number}
          </strong>
          <span>{run.repo}</span>
        </div>
        <div className="spacer" />
        <button className="text" onClick={() => openUrl(log?.webUrl ?? run.webUrl)}>
          Open <IconExternal />
        </button>
      </header>

      <div className="log-tools">
        {log && log.steps.length > 1 ? (
          <label className="step-pick">
            <StatusIcon status={log.steps.find((s) => s.id === log.stepId)?.status ?? "skipped"} />
            <select value={log.stepId} onChange={(e) => setStep(e.target.value)} aria-label="Step">
              {log.steps.map((s) => (
                <option key={s.id} value={s.id}>
                  {s.status === "failed" ? "✕ " : ""}
                  {s.name}
                </option>
              ))}
            </select>
          </label>
        ) : (
          <span className="step-name">{log?.step}</span>
        )}
        <div className="spacer" />
        {errorLines.length > 0 && (
          <button className="tool" onClick={() => jump(1)} title="Next error (⌘G), previous (⇧⌘G)">
            Error {errorIndex + 1} of {errorLines.length}
          </button>
        )}
        <button className="tool" onClick={copy} disabled={!log} title="Copy the log">
          <IconCopy /> {copied ? "Copied" : "Copy"}
        </button>
      </div>

      {error && <div className="banner error">{error}</div>}
      {!log && !error && <p className="empty">Loading log…</p>}
      {log && (
        <div className="log" ref={bodyRef}>
          {lines.map((line, i) => (
            <div
              key={i}
              ref={(el) => {
                lineRefs.current[i] = el;
              }}
              className={`log-line${isErrorLine(line) ? " err" : ""}${i === current ? " current" : ""}`}
            >
              {line || " "}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
