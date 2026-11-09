import type { ReactNode } from "react";
import { Fragment, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api, errorText } from "./api";
import type { LogTail, Run } from "./types";
import type { LogLine } from "./model";
import { errorContext, isActive, parseLog } from "./model";
import { IconBack, IconChevron, IconCopy, IconExpand, IconExternal, IconSearch, IconWrap, StatusIcon } from "./icons";

// GitHub only serves a job's log once it finishes, so its refresh just watches for that.
const LIVE_REFRESH_MS = { github: 15_000, bitbucket: 5_000 };

function loadWrap() {
  try {
    return localStorage.getItem("logWrap") !== "0";
  } catch {
    return true;
  }
}

function highlight(text: string, query: string) {
  if (!query) return text || " ";
  const lower = text.toLowerCase();
  const q = query.toLowerCase();
  const parts: ReactNode[] = [];
  let from = 0;
  for (let at = lower.indexOf(q); at >= 0; at = lower.indexOf(q, from)) {
    parts.push(text.slice(from, at), <mark key={at}>{text.slice(at, at + q.length)}</mark>);
    from = at + q.length;
  }
  parts.push(text.slice(from));
  return parts;
}

interface Props {
  run: Run;
  // Absent in the standalone log window, which has no list to go back to.
  onBack?: () => void;
  initialStep?: string;
}

export function LogView({ run, onBack, initialStep }: Props) {
  const expanded = !onBack;
  const [log, setLog] = useState<LogTail | null>(null);
  const [step, setStep] = useState<string | undefined>(initialStep);
  const [expandError, setExpandError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [query, setQuery] = useState("");
  const [cursor, setCursor] = useState(0);
  const [errorsOnly, setErrorsOnly] = useState(false);
  const [wrap, setWrap] = useState(loadWrap);
  const [toggled, setToggled] = useState<Set<number>>(new Set());
  const lineRefs = useRef(new Map<number, HTMLDivElement>());
  const bodyRef = useRef<HTMLDivElement>(null);
  const findRef = useRef<HTMLInputElement>(null);
  const positioned = useRef<string | null>(null);
  const stickToBottom = useRef(false);
  const fetchLog = useRef(() => {});
  const live = isActive(run);
  const lastLive = useRef({ key: run.key, live });

  useEffect(() => {
    setLog(null);
    setError(null);
    setToggled(new Set());
    positioned.current = null;
    let stop = false;
    fetchLog.current = () =>
      api
        .log(run.key, step)
        .then((l) => {
          if (stop) return;
          const body = bodyRef.current;
          stickToBottom.current = !!body && body.scrollHeight - body.scrollTop - body.clientHeight < 24;
          setLog(l);
          setError(null);
        })
        .catch((e) => !stop && setError(errorText(e)));
    fetchLog.current();
    return () => {
      stop = true;
      fetchLog.current = () => {};
    };
  }, [run.key, step]);

  // Kept apart from the load above so a run finishing refreshes in place instead of resetting the view.
  useEffect(() => {
    const finished = lastLive.current.key === run.key && lastLive.current.live && !live;
    lastLive.current = { key: run.key, live };
    if (finished) fetchLog.current();
    if (!live) return;
    const id = window.setInterval(async () => {
      // The popover only hides on blur, so skip refreshes nobody can see.
      if (document.visibilityState === "hidden" || !(await getCurrentWindow().isVisible())) return;
      fetchLog.current();
    }, LIVE_REFRESH_MS[run.provider]);
    return () => window.clearInterval(id);
  }, [run.key, run.provider, live]);

  const parsed = useMemo(() => parseLog(log?.text ?? ""), [log]);
  const { lines, groups } = parsed;
  const q = query.trim();

  // Groups start collapsed unless they hold an error; a click flips that default.
  const isOpen = (id: number) => groups[id].hasError !== toggled.has(id);
  const hidden = (l: LogLine) => l.group !== null && !q && !errorsOnly && !isOpen(l.group);

  const targets = useMemo(() => {
    const want = q ? (l: LogLine) => l.text.toLowerCase().includes(q.toLowerCase()) : (l: LogLine) => l.kind === "error";
    return lines.filter(want).map((l) => l.n);
  }, [lines, q]);
  const errorCount = useMemo(() => lines.filter((l) => l.kind === "error").length, [lines]);
  const current = targets[Math.min(cursor, targets.length - 1)];

  useEffect(() => setCursor(0), [q]);
  useEffect(() => {
    if (!errorCount) setErrorsOnly(false);
  }, [errorCount]);

  const reveal = (n: number) => {
    const line = lines[n - 1];
    if (line && line.group !== null && !isOpen(line.group)) {
      setToggled((t) => new Set(t).add(line.group!));
    }
    requestAnimationFrame(() => lineRefs.current.get(n)?.scrollIntoView({ block: "center" }));
  };

  // First load of a step opens on the first error; a live refresh keeps the reader's place.
  useLayoutEffect(() => {
    if (!log) return;
    const body = bodyRef.current;
    if (positioned.current !== log.stepId) {
      positioned.current = log.stepId;
      setCursor(0);
      // Group ids belong to one step's log; a live run can move on to another step.
      setToggled(new Set());
      const first = lines.find((l) => l.kind === "error");
      if (first) reveal(first.n);
      else if (body) body.scrollTop = body.scrollHeight;
    } else if (stickToBottom.current && body) {
      body.scrollTop = body.scrollHeight;
    }
  }, [log]);

  const jump = (delta: number) => {
    if (!targets.length) return;
    const next = (cursor + delta + targets.length) % targets.length;
    setCursor(next);
    reveal(targets[next]);
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!e.metaKey) return;
      const key = e.key.toLowerCase();
      if (key === "e" && e.shiftKey) {
        e.preventDefault();
        if (!expanded) expand();
      } else if (key === "w" && expanded) {
        e.preventDefault();
        getCurrentWindow().close();
      } else if (key === "g") {
        e.preventDefault();
        jump(e.shiftKey ? -1 : 1);
      } else if (key === "f") {
        e.preventDefault();
        findRef.current?.select();
      } else if (key === "e") {
        e.preventDefault();
        if (errorCount) setErrorsOnly((v) => !v);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const expand = () => {
    setExpandError(null);
    api.openLogWindow(run.key, log?.stepId || step).catch((e) => setExpandError(errorText(e)));
  };

  const toggleWrap = () => {
    setWrap((v) => {
      try {
        localStorage.setItem("logWrap", v ? "0" : "1");
      } catch {}
      return !v;
    });
  };

  const rows = errorsOnly ? errorContext(lines) : lines.map((_, i) => i);
  const copyText = () =>
    errorsOnly ? rows.map((i) => (i === null ? "…" : lines[i].text)).join("\n") : lines.map((l) => l.text).join("\n");

  const copy = async () => {
    const text = copyText();
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      const area = document.createElement("textarea");
      area.value = text;
      document.body.appendChild(area);
      area.select();
      document.execCommand("copy");
      area.remove();
    }
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1500);
  };

  const groupHeader = (l: LogLine) => {
    const g = groups[l.group!];
    const open = isOpen(g.id) || !!q;
    return (
      <button
        key={`g${g.id}`}
        className={`log-group${open ? " open" : ""}${g.hasError ? " has-error" : ""}`}
        onClick={() => setToggled((t) => {
          const next = new Set(t);
          if (!next.delete(g.id)) next.add(g.id);
          return next;
        })}
        aria-expanded={open}
      >
        <span className="ln">{g.n + (log?.skipped ?? 0)}</span>
        <IconChevron dir={open ? "down" : "right"} />
        <span className="log-group-title">{g.title}</span>
        <span className="log-group-size">{g.size} {g.size === 1 ? "line" : "lines"}</span>
      </button>
    );
  };

  const renderLine = (l: LogLine) => (
    <div
      key={l.n}
      ref={(el) => {
        if (el) lineRefs.current.set(l.n, el);
        else lineRefs.current.delete(l.n);
      }}
      className={`log-line ${l.kind}${l.n === current ? " current" : ""}${l.group !== null ? " grouped" : ""}`}
    >
      <span className="ln">{l.n + (log?.skipped ?? 0)}</span>
      <span className="lt">{highlight(l.text, q)}</span>
    </div>
  );

  const stepInfo = log?.steps.find((s) => s.id === log.stepId);
  const navLabel = q ? (targets.length ? `${Math.min(cursor, targets.length - 1) + 1}/${targets.length}` : "0/0") : null;

  return (
    <div className={`app${expanded ? " expanded" : ""}`}>
      <header className="bar">
        {onBack && (
          <button className="icon" title="Back (esc)" aria-label="Back" onClick={onBack}>
            <IconBack />
          </button>
        )}
        <StatusIcon status={run.status} />
        <div className="log-title">
          <strong>
            {run.title} <span className="num">#{run.number}</span>
          </strong>
          <span>{[run.repo, run.branch].filter(Boolean).join(" · ")}</span>
        </div>
        <div className="spacer" />
        {live && <span className="live-pill">Live</span>}
        <button className="text" onClick={() => openUrl(log?.webUrl ?? run.webUrl)} title="Open in browser">
          Open <IconExternal />
        </button>
        {!expanded && (
          <button className="icon" onClick={expand} title="Open in a larger window (⇧⌘E)" aria-label="Open in a larger window">
            <IconExpand />
          </button>
        )}
      </header>

      <div className="log-tools">
        {log && log.steps.length > 1 ? (
          <label className="step-pick">
            <StatusIcon status={stepInfo?.status ?? "skipped"} />
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
        {errorCount > 0 && (
          <div className="error-nav" role="group" aria-label="Errors">
            <button onClick={() => jump(-1)} title="Previous error (⇧⌘G)" aria-label="Previous error" disabled={!!q}>
              <IconChevron dir="up" />
            </button>
            <span>
              {q ? `${errorCount} ${errorCount === 1 ? "error" : "errors"}` : `Error ${Math.min(cursor, targets.length - 1) + 1} of ${errorCount}`}
            </span>
            <button onClick={() => jump(1)} title="Next error (⌘G)" aria-label="Next error" disabled={!!q}>
              <IconChevron dir="down" />
            </button>
          </div>
        )}
      </div>

      <div className="log-tools find-row">
        <label className="find">
          <IconSearch />
          <input
            ref={findRef}
            value={query}
            placeholder="Find in log"
            aria-label="Find in log"
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                jump(e.shiftKey ? -1 : 1);
              } else if (e.key === "Escape" && query) {
                e.stopPropagation();
                setQuery("");
              }
            }}
          />
          {navLabel && <span className="find-count">{navLabel}</span>}
        </label>
        <button
          className={`tool${errorsOnly ? " on" : ""}`}
          onClick={() => setErrorsOnly((v) => !v)}
          disabled={!errorCount}
          title="Show only errors and the lines around them (⌘E)"
          aria-pressed={errorsOnly}
        >
          Errors only
        </button>
        <button className={`tool icon-only${wrap ? " on" : ""}`} onClick={toggleWrap} title="Wrap long lines" aria-pressed={wrap}>
          <IconWrap />
        </button>
        <button
          className="tool icon-only"
          onClick={copy}
          disabled={!log}
          title={copied ? "Copied" : errorsOnly ? "Copy the errors" : "Copy the log"}
          aria-label={copied ? "Copied" : errorsOnly ? "Copy the errors" : "Copy the log"}
        >
          {copied ? "✓" : <IconCopy />}
        </button>
      </div>

      {error && <div className="banner error">{error}</div>}
      {expandError && <div className="banner error">{expandError}</div>}
      {!log && !error && (
        <div className="log-loading" aria-label="Loading log">
          {Array.from({ length: 9 }, (_, i) => (
            <span key={i} style={{ width: `${30 + ((i * 37) % 60)}%` }} />
          ))}
        </div>
      )}
      {log && (
        <div className={`log${wrap ? " wrap" : ""}`} ref={bodyRef}>
          {log.skipped > 0 && !errorsOnly && (
            <button className="log-skipped" onClick={() => openUrl(log.webUrl)}>
              {log.skipped.toLocaleString()} earlier lines not shown · Open the full log <IconExternal />
            </button>
          )}
          {rows.map((i, k) => {
            if (i === null) {
              const gap = (rows[k + 1] ?? 0) - (rows[k - 1] ?? 0) - 1;
              return (
                <div key={`gap${k}`} className="log-gap">
                  ⋯ {gap} {gap === 1 ? "line" : "lines"}
                </div>
              );
            }
            const l = lines[i];
            const startsGroup = !errorsOnly && !q && l.group !== null && (i === 0 || lines[i - 1].group !== l.group);
            if (hidden(l)) return startsGroup ? groupHeader(l) : null;
            return (
              <Fragment key={l.n}>
                {startsGroup && groupHeader(l)}
                {renderLine(l)}
              </Fragment>
            );
          })}
          {lines.length === 1 && !lines[0].text && <p className="empty">This step has no output.</p>}
          {live && <div className="log-tail-live">Waiting for more output…</div>}
        </div>
      )}
    </div>
  );
}

// Root of a standalone log window: tracks the run itself so the status and live tail stay current.
export function LogWindow({ runKey, step }: { runKey: string; step?: string }) {
  const [run, setRun] = useState<Run | null>(null);
  const [missing, setMissing] = useState(false);

  useEffect(() => {
    const load = () =>
      api.run(runKey).then((found) => {
        if (found) setRun(found);
        else setMissing(true);
      });
    load();
    // The first sweep emits one event per repo, so coalesce reloads.
    let timer: number | undefined;
    const unlisten = listen("runs-updated", () => {
      window.clearTimeout(timer);
      timer = window.setTimeout(load, 300);
    });
    return () => {
      unlisten.then((f) => f());
      window.clearTimeout(timer);
    };
  }, [runKey]);

  if (run) return <LogView run={run} initialStep={step} />;
  return <p className="empty">{missing ? "This run is no longer in the list." : "Loading…"}</p>;
}
