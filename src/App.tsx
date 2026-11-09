import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api, errorText, label } from "./api";
import type { ProviderStatus, Run, Settings } from "./types";
import type { Pending, StateFilter } from "./model";
import { ago, buildSections, isActive, settled, visibleOrder } from "./model";
import { RunRow } from "./RunRow";
import { SettingsView } from "./SettingsView";
import { LogView } from "./LogView";
import { IconGear, IconMute, IconPin, IconRefresh, IconSearch } from "./icons";

type View = { kind: "list" } | { kind: "settings" } | { kind: "log"; run: Run };

function loadMineOnly() {
  try {
    return localStorage.getItem("mineOnly") !== "0";
  } catch {
    return true;
  }
}

export default function App() {
  const [runs, setRuns] = useState<Run[]>([]);
  const [statuses, setStatuses] = useState<ProviderStatus[]>([]);
  const [view, setView] = useState<View>({ kind: "list" });
  const [mineOnly, setMineOnly] = useState(loadMineOnly);
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<StateFilter>("all");
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const [pending, setPending] = useState<Record<string, Pending>>({});
  const [confirmKey, setConfirmKey] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [now, setNow] = useState(Date.now);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [showMuted, setShowMuted] = useState(false);
  const searchRef = useRef<HTMLInputElement>(null);
  const runsRef = useRef(runs);
  runsRef.current = runs;
  const rowRefs = useRef(new Map<string, HTMLDivElement>());

  const loadRuns = useCallback(() => api.runs().then(setRuns).catch((e) => setError(errorText(e))), []);
  const loadStatus = useCallback(() => api.status().then(setStatuses), []);
  const loadSettings = useCallback(() => api.settings().then(setSettings), []);

  useEffect(() => {
    loadRuns();
    loadStatus();
    loadSettings();
    // The first sweep emits one event per repo, so coalesce reloads.
    let timer: number | undefined;
    const unlisten = [
      listen("runs-updated", () => {
        window.clearTimeout(timer);
        timer = window.setTimeout(loadRuns, 300);
      }),
      listen("status-changed", loadStatus),
      listen("show-settings", () => setView({ kind: "settings" })),
      listen<string>("focus-run", ({ payload: key }) => {
        setView({ kind: "list" });
        setQuery("");
        setFilter("all");
        setSelectedKey(key);
        // A notification can be for someone else's run when "notify for everyone" is on.
        if (runsRef.current.some((r) => r.key === key && !r.mine)) toggleMine(false);
      }),
      listen("show-failing", () => {
        setView({ kind: "list" });
        setQuery("");
        setFilter("failing");
      }),
      getCurrentWindow().onFocusChanged(({ payload: focused }) => {
        if (focused) searchRef.current?.select();
      }),
    ];
    return () => {
      unlisten.forEach((p) => p.then((f) => f()));
      window.clearTimeout(timer);
    };
  }, [loadRuns, loadStatus, loadSettings]);

  // Freshness and rate limits change on every poll without an event, so re-read them.
  useEffect(() => {
    const id = window.setInterval(loadStatus, 5000);
    return () => window.clearInterval(id);
  }, [loadStatus]);

  const flags = useMemo(
    () => ({ pinned: new Set(settings?.pinned), muted: new Set(settings?.muted), showMuted }),
    [settings, showMuted],
  );
  const sections = useMemo(
    () => buildSections(runs, mineOnly, query, now, flags),
    [runs, mineOnly, query, now, flags],
  );
  const order = useMemo(() => visibleOrder(sections, filter), [sections, filter]);
  const selected = order.find((r) => r.key === selectedKey) ?? order[0];

  // Tick every second while something is running so elapsed times stay live.
  const anyActive = runs.some(isActive) || Object.keys(pending).length > 0;
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), anyActive ? 1000 : 30_000);
    return () => window.clearInterval(id);
  }, [anyActive]);

  useEffect(() => {
    setPending((current) => {
      const left = Object.fromEntries(Object.entries(current).filter(([, p]) => !settled(p, runs, now)));
      return Object.keys(left).length === Object.keys(current).length ? current : left;
    });
  }, [runs, now]);

  useEffect(() => {
    if (selected) rowRefs.current.get(selected.key)?.scrollIntoView({ block: "nearest" });
  }, [selected]);

  const setFlag = (id: string, flag: "pinned" | "muted", on: boolean) =>
    act(async () => {
      await api.setRepoFlag(id, flag, on);
      await loadSettings();
    });

  function toggleMine(value: boolean) {
    setMineOnly(value);
    try {
      localStorage.setItem("mineOnly", value ? "1" : "0");
    } catch {}
  }

  const act = async (fn: () => Promise<void>) => {
    setError(null);
    try {
      await fn();
    } catch (e) {
      setError(errorText(e));
    }
  };

  const track = async (run: Run, kind: Pending["kind"], call: () => Promise<void>) => {
    setError(null);
    setPending((p) => ({ ...p, [run.key]: { kind, at: Date.now(), run } }));
    try {
      await call();
    } catch (e) {
      setPending(({ [run.key]: _, ...rest }) => rest);
      setError(errorText(e));
    }
  };
  const rerun = (run: Run, failedOnly: boolean) => track(run, "rerun", () => api.rerun(run.key, failedOnly));
  const cancel = (run: Run) => {
    setConfirmKey(null);
    track(run, "cancel", () => api.cancel(run.key));
  };
  const canRerun = (run: Run) => !isActive(run) && run.status !== "waiting" && !pending[run.key];

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (view.kind !== "list") {
        if (e.key === "Escape") setView({ kind: "list" });
        return;
      }
      const mod = e.metaKey;
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        if (!order.length) return;
        const i = selected ? order.indexOf(selected) : -1;
        const next = e.key === "ArrowDown" ? Math.min(i + 1, order.length - 1) : Math.max(i - 1, 0);
        setSelectedKey(order[next].key);
        setConfirmKey(null);
      } else if (e.key === "Enter") {
        // Leave Enter alone on buttons and other controls so they still activate normally.
        const target = e.target as HTMLElement;
        if (target !== searchRef.current && target !== document.body) return;
        e.preventDefault();
        if (selected && confirmKey === selected.key) cancel(selected);
        else if (selected) openUrl(selected.webUrl);
      } else if (e.key === "Escape") {
        e.preventDefault();
        if (confirmKey) setConfirmKey(null);
        else if (query) setQuery("");
        else getCurrentWindow().hide();
      } else if (mod && e.key.toLowerCase() === "r") {
        // Always swallow ⌘R so the webview never reloads.
        e.preventDefault();
        if (selected && canRerun(selected)) {
          const failedOnly = selected.provider === "github" && selected.status === "failed" && !e.shiftKey;
          rerun(selected, failedOnly);
        }
      } else if (mod && e.key.toLowerCase() === "l") {
        e.preventDefault();
        if (selected) setView({ kind: "log", run: selected });
      } else if (mod && e.key === ".") {
        e.preventDefault();
        if (selected && isActive(selected) && !pending[selected.key]) setConfirmKey(selected.key);
      } else if (mod && e.key === ",") {
        e.preventDefault();
        setView({ kind: "settings" });
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  if (settings && !settings.onboarded) {
    return (
      <SettingsView
        welcome
        statuses={statuses}
        onBack={async () => {
          await api.setOnboarded();
          loadSettings();
        }}
      />
    );
  }
  if (view.kind === "settings") {
    return (
      <SettingsView
        statuses={statuses}
        onBack={() => {
          setView({ kind: "list" });
          loadSettings();
        }}
      />
    );
  }
  if (view.kind === "log") {
    return <LogView run={view.run} onBack={() => setView({ kind: "list" })} />;
  }

  const needsSetup = statuses.filter((s) => s.state === "notConfigured" || s.state === "error");
  const offline = statuses.some((s) => s.state === "offline");
  const lastPoll = statuses.map((s) => s.lastPoll).filter((t): t is string => !!t).sort().pop();
  const lowRate = statuses.filter((s) => s.rate && s.rate.remaining < s.rate.limit * 0.1);
  const warnings = statuses.flatMap((s) => s.warnings);
  const connecting = statuses.some((s) => s.state === "connecting");
  const toggleFilter = (f: StateFilter) => setFilter((cur) => (cur === f ? "all" : f));

  const row = (run: Run, showRepo: boolean) => (
    <RunRow
      key={run.key}
      ref={(el) => {
        if (el) rowRefs.current.set(run.key, el);
        else rowRefs.current.delete(run.key);
      }}
      run={run}
      now={now}
      showRepo={showRepo}
      selected={selected?.key === run.key}
      pending={pending[run.key]}
      confirming={confirmKey === run.key}
      onSelect={() => setSelectedKey(run.key)}
      onOpen={() => openUrl(run.webUrl)}
      onLog={() => setView({ kind: "log", run })}
      onRerun={(failedOnly) => rerun(run, failedOnly)}
      onCancel={() => setConfirmKey(run.key)}
      onConfirmCancel={() => cancel(run)}
      onKeepRunning={() => setConfirmKey(null)}
    />
  );

  const empty = order.length === 0;
  return (
    <div className="app">
      <header className="bar">
        <label className="search">
          <IconSearch />
          <input
            ref={searchRef}
            autoFocus
            value={query}
            placeholder="Filter repos, branches…"
            onChange={(e) => {
              setQuery(e.target.value);
              setSelectedKey(null);
            }}
            aria-label="Filter runs"
          />
        </label>
        <div className="segmented" role="group" aria-label="Show runs">
          <button className={mineOnly ? "on" : ""} onClick={() => toggleMine(true)}>
            Mine
          </button>
          <button className={mineOnly ? "" : "on"} onClick={() => toggleMine(false)}>
            All
          </button>
        </div>
        <button className="icon" title="Refresh" aria-label="Refresh" onClick={() => act(api.refresh)}>
          <IconRefresh />
        </button>
        <button className="icon" title="Settings (⌘,)" aria-label="Settings" onClick={() => setView({ kind: "settings" })}>
          <IconGear />
        </button>
      </header>

      <nav className="summary" aria-label="Status filters">
        {sections.failing.length > 0 ? (
          <button className={`chip failed${filter === "failing" ? " on" : ""}`} onClick={() => toggleFilter("failing")}>
            {sections.failing.length} failing
          </button>
        ) : (
          <span className="chip ok">Nothing failing</span>
        )}
        {sections.running.length > 0 && (
          <button className={`chip running${filter === "running" ? " on" : ""}`} onClick={() => toggleFilter("running")}>
            {sections.running.length} running
          </button>
        )}
        <span className="chip muted">{sections.passing} passing</span>
        <span className={`freshness${offline ? " offline" : ""}`}>
          {offline ? "Offline" : lastPoll ? `Updated ${ago(lastPoll, now)}` : ""}
        </span>
      </nav>

      {needsSetup.length > 0 && (
        <button className="banner" onClick={() => setView({ kind: "settings" })}>
          {needsSetup
            .map((s) => `${label(s.provider)}: ${s.state === "error" ? s.message : "not connected"}`)
            .join(" · ")}
        </button>
      )}
      {lowRate.map((s) => (
        <div key={s.provider} className="banner">
          {label(s.provider)} rate limit low: {s.rate!.remaining.toLocaleString()} of {s.rate!.limit.toLocaleString()} left
          {s.rate!.resetSecs != null ? `, resets in ${Math.ceil(s.rate!.resetSecs / 60)}m` : " this hour"}. Polling has slowed.
        </div>
      ))}
      {warnings.map((w) => (
        <button key={w} className="banner" onClick={() => setView({ kind: "settings" })}>
          {w}
        </button>
      ))}
      {error && (
        <div className="banner error" onClick={() => setError(null)}>
          {error}
        </div>
      )}

      <main className="list" role="listbox" aria-label="Runs">
        {empty && (
          <p className="empty">
            {connecting
              ? "Loading runs…"
              : query
                ? `Nothing matches “${query}”.`
                : filter !== "all"
                  ? `Nothing ${filter} right now.`
                  : mineOnly
                    ? "No runs of yours in the last 7 days."
                    : "No runs in the last 7 days."}
          </p>
        )}
        {filter !== "running" && sections.failing.length > 0 && (
          <section className="group attention">
            <h2>Needs attention</h2>
            {sections.failing.map((r) => row(r, true))}
          </section>
        )}
        {filter !== "failing" && sections.running.length > 0 && (
          <section className="group">
            <h2>Running</h2>
            {sections.running.map((r) => row(r, true))}
          </section>
        )}
        {filter === "all" &&
          sections.groups.map((g) => (
            <section key={g.id} className={`group${g.muted ? " is-muted" : ""}`}>
              <h2>
                <span className={`provider ${g.provider}`}>{g.provider === "github" ? "GH" : "BB"}</span>
                <span className="repo-name">{g.repo}</span>
                <span className="repo-tools">
                  <button
                    className={g.pinned ? "on" : ""}
                    title={g.pinned ? "Unpin" : "Pin to top"}
                    aria-label={g.pinned ? `Unpin ${g.repo}` : `Pin ${g.repo}`}
                    onClick={() => setFlag(g.id, "pinned", !g.pinned)}
                  >
                    <IconPin filled={g.pinned} />
                  </button>
                  <button
                    className={g.muted ? "on" : ""}
                    title={g.muted ? "Unmute" : "Mute: hide and stop notifications"}
                    aria-label={g.muted ? `Unmute ${g.repo}` : `Mute ${g.repo}`}
                    onClick={() => setFlag(g.id, "muted", !g.muted)}
                  >
                    <IconMute muted={g.muted} />
                  </button>
                </span>
              </h2>
              {g.runs.map((r) => row(r, false))}
            </section>
          ))}
        {filter === "all" && sections.mutedRepos > 0 && (
          <button className="show-muted" onClick={() => setShowMuted((v) => !v)}>
            {showMuted
              ? "Hide muted repos"
              : `${sections.mutedRepos} muted ${sections.mutedRepos === 1 ? "repo" : "repos"} hidden · Show`}
          </button>
        )}
      </main>

      <footer className="hints" aria-hidden>
        <span>↑↓ select</span>
        <span>↵ open</span>
        <span>⌘L log</span>
        <span>⌘R re-run</span>
        <span>⌘. cancel</span>
        <span>esc close</span>
      </footer>
    </div>
  );
}
