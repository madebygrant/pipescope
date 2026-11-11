import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Menu } from "@tauri-apps/api/menu";
import type { MenuItemOptions } from "@tauri-apps/api/menu";
import { api, errorText, label } from "./api";
import type { PendingDeployment, ProviderStatus, Run, Settings } from "./types";
import type { Pending, StateFilter } from "./model";
import { ago, buildSections, isActive, progress, settled, typicalDurations, visibleOrder } from "./model";
import { RunRow } from "./RunRow";
import type { Confirm } from "./RunRow";
import { ClientsView } from "./ClientsView";
import { SettingsView } from "./SettingsView";
import { LogView } from "./LogView";
import { IconClients, IconGear, IconMute, IconPin, IconRefresh, IconTag } from "./icons";

type View = { kind: "list" } | { kind: "settings" } | { kind: "clients" } | { kind: "log"; run: Run };

const pad = (n: number) => String(n).padStart(2, "0");

async function copyText(text: string) {
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
}

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
  const [confirm, setConfirm] = useState<{ key: string; action: Confirm } | null>(null);
  const [deployments, setDeployments] = useState<Record<string, PendingDeployment[]>>({});
  const [editingClient, setEditingClient] = useState<string | null>(null);
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
  const typical = useMemo(() => typicalDurations(runs), [runs]);
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
  const confirmAction = (run: Run, action: Confirm) => {
    setConfirm(null);
    if (action === "cancel") {
      track(run, "cancel", () => api.cancel(run.key));
      return;
    }
    track(run, "review", async () => {
      await api.reviewDeployments(run.key, action === "approve");
      setDeployments(({ [run.key]: _, ...rest }) => rest);
    });
  };
  const toggleWatch = (run: Run) =>
    act(async () => {
      await api.watch(run.key, !run.watched);
      await loadRuns();
    });
  const openStep = (run: Run) =>
    act(async () => {
      await openUrl(await api.manualStepUrl(run.key));
    });
  const setClient = (id: string, client: string) =>
    act(async () => {
      setEditingClient(null);
      await api.setRepoClient(id, client.trim() || null);
      await loadSettings();
    });

  // Who can approve is only known by asking. Rows show approve icons unselected, so ask for every
  // visible waiting GitHub run; `asked` stops the per-second re-render from asking again mid-request.
  const asked = useRef(new Set<string>());
  useEffect(() => {
    for (const run of order) {
      if (run.status !== "waiting" || run.provider !== "github" || asked.current.has(run.key)) continue;
      const key = run.key;
      asked.current.add(key);
      api
        .pendingDeployments(key)
        .then((d) => setDeployments((all) => ({ ...all, [key]: d })))
        .catch(() => setDeployments((all) => ({ ...all, [key]: [] })));
    }
  }, [order]);

  // Drop lookups for runs that have moved on, so a later wait is looked up afresh.
  useEffect(() => {
    for (const key of asked.current) {
      if (!runs.some((r) => r.key === key && r.status === "waiting")) asked.current.delete(key);
    }
    setDeployments((all) => {
      const keep = Object.fromEntries(
        Object.entries(all).filter(([key]) => runs.some((r) => r.key === key && r.status === "waiting")),
      );
      return Object.keys(keep).length === Object.keys(all).length ? all : keep;
    });
  }, [runs]);
  const canRerun = (run: Run) => !isActive(run) && run.status !== "waiting" && !pending[run.key];

  // Every action for a run, including the ones its two row icons leave out.
  const showRunMenu = async (run: Run) => {
    setSelectedKey(run.key);
    const finished = !isActive(run) && run.status !== "waiting";
    const failedOnGithub = run.status === "failed" && run.provider === "github";
    const approvable = deployments[run.key]?.some((d) => d.canApprove) ?? false;
    const busy = !!pending[run.key];
    const ask = (action: Confirm) => () => setConfirm({ key: run.key, action });
    const groups: MenuItemOptions[][] = [
      [
        { text: "Open in Browser", accelerator: "Enter", action: () => openUrl(run.webUrl) },
        { text: "Show Log", accelerator: "CmdOrCtrl+L", action: () => setView({ kind: "log", run }) },
      ],
      [
        ...(approvable
          ? [
              { text: "Approve Deployment…", enabled: !busy, action: ask("approve") },
              { text: "Reject Deployment…", enabled: !busy, action: ask("reject") },
            ]
          : []),
        ...(run.status === "waiting" && run.provider === "bitbucket"
          ? [{ text: "Open Manual Step", action: () => openStep(run) }]
          : []),
        ...(failedOnGithub
          ? [{ text: "Re-run Failed Jobs", accelerator: "CmdOrCtrl+R", enabled: canRerun(run), action: () => rerun(run, true) }]
          : []),
        ...(finished
          ? [
              {
                text: run.provider === "bitbucket" ? "Run Pipeline Again" : "Re-run All Jobs",
                accelerator: failedOnGithub ? "CmdOrCtrl+Shift+R" : "CmdOrCtrl+R",
                enabled: canRerun(run),
                action: () => rerun(run, false),
              },
            ]
          : []),
        ...(!finished
          ? [{ text: run.watched ? "Stop Watching" : "Notify When Finished", accelerator: "CmdOrCtrl+D", action: () => toggleWatch(run) }]
          : []),
        ...(isActive(run) ? [{ text: "Cancel Run…", accelerator: "CmdOrCtrl+.", enabled: !busy, action: ask("cancel") }] : []),
      ],
      [{ text: "Copy Link", action: () => copyText(run.webUrl) }],
    ];
    const items = groups
      .filter((g) => g.length)
      .flatMap((g, i) => (i ? [{ item: "Separator" as const }, ...g] : g));
    try {
      const menu = await Menu.new({ items });
      await menu.popup();
    } catch (e) {
      setError(errorText(e));
    }
  };

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
        setConfirm(null);
      } else if (e.key === "Enter") {
        // Leave Enter alone on buttons and other controls so they still activate normally.
        const target = e.target as HTMLElement;
        if (target !== searchRef.current && target !== document.body) return;
        e.preventDefault();
        if (selected && confirm?.key === selected.key) confirmAction(selected, confirm.action);
        else if (selected) openUrl(selected.webUrl);
      } else if (e.key === "Escape") {
        e.preventDefault();
        if (confirm) setConfirm(null);
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
        if (selected && isActive(selected) && !pending[selected.key]) setConfirm({ key: selected.key, action: "cancel" });
      } else if (mod && e.key.toLowerCase() === "d") {
        e.preventDefault();
        if (selected && (isActive(selected) || selected.status === "waiting")) toggleWatch(selected);
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
  if (view.kind === "clients") {
    return (
      <ClientsView runs={runs} settings={settings} onBack={() => setView({ kind: "list" })} onClientsChanged={loadSettings} />
    );
  }
  if (view.kind === "log") {
    // Look the run up again so the log view sees status changes and knows when to stop tailing.
    const run = runs.find((r) => r.key === view.run.key) ?? view.run;
    return <LogView run={run} onBack={() => setView({ kind: "list" })} />;
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
      progress={isActive(run) ? progress(run, now, typical) : undefined}
      selected={selected?.key === run.key}
      pending={pending[run.key]}
      confirming={confirm?.key === run.key ? confirm.action : null}
      deployments={deployments[run.key]}
      onSelect={() => setSelectedKey(run.key)}
      onOpen={() => openUrl(run.webUrl)}
      onLog={() => setView({ kind: "log", run })}
      onRerun={(failedOnly) => rerun(run, failedOnly)}
      onAsk={(action) => setConfirm({ key: run.key, action })}
      onConfirm={() => confirm && confirmAction(run, confirm.action)}
      onDismiss={() => setConfirm(null)}
      onWatch={() => toggleWatch(run)}
      onOpenStep={() => openStep(run)}
      onMenu={(e) => {
        e.preventDefault();
        showRunMenu(run);
      }}
    />
  );

  const empty = order.length === 0;
  const showFailing = filter !== "running" && sections.failing.length > 0;
  const showRunning = filter !== "failing" && sections.running.length > 0;
  const firstRepoSection = 1 + Number(showFailing) + Number(showRunning);
  return (
    <div className="app">
      <header className="bar">
        <span className="brand">Pipescope</span>
        <div className="spacer" />
        <div className="who" role="group" aria-label="Show runs">
          <button className={mineOnly ? "on" : ""} aria-pressed={mineOnly} onClick={() => toggleMine(true)}>
            Mine
          </button>
          <button className={mineOnly ? "" : "on"} aria-pressed={!mineOnly} onClick={() => toggleMine(false)}>
            All
          </button>
        </div>
        <button className="icon" title="Clients" aria-label="Clients" onClick={() => setView({ kind: "clients" })}>
          <IconClients />
        </button>
        <button className="icon" title="Settings (⌘,)" aria-label="Settings" onClick={() => setView({ kind: "settings" })}>
          <IconGear />
        </button>
      </header>

      <nav className="counts" aria-label="Status filters">
        <button
          className={`count failing${filter === "failing" ? " on" : ""}`}
          onClick={() => toggleFilter("failing")}
          disabled={!sections.failing.length && filter !== "failing"}
          aria-pressed={filter === "failing"}
          title="Show only failing runs"
        >
          <span className="count-n">{pad(sections.failing.length)}</span>
          <span className="count-label">Failing</span>
        </button>
        <button
          className={`count running${filter === "running" ? " on" : ""}`}
          onClick={() => toggleFilter("running")}
          disabled={!sections.running.length && filter !== "running"}
          aria-pressed={filter === "running"}
          title="Show only running runs"
        >
          <span className="count-n">{pad(sections.running.length)}</span>
          <span className="count-label">Running</span>
        </button>
        <div className="count passed">
          <span className="count-n">{pad(sections.passing)}</span>
          <span className="count-label">Passed</span>
        </div>
      </nav>

      <div className="filter">
        <label>
          <span className="filter-label">Filter</span>
          <input
            ref={searchRef}
            autoFocus
            value={query}
            placeholder="repo, branch, author"
            onChange={(e) => {
              setQuery(e.target.value);
              setSelectedKey(null);
            }}
            aria-label="Filter runs"
          />
        </label>
        <button
          className={`freshness${offline ? " offline" : ""}`}
          title={lastPoll ? `Updated ${ago(lastPoll, now)}. Refresh now` : "Refresh now"}
          onClick={() => act(api.refresh)}
        >
          <IconRefresh />
          {offline ? "Offline" : lastPoll ? ago(lastPoll, now) : "Refresh"}
        </button>
      </div>

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
        {showFailing && (
          <section className="group attention">
            <h2>
              <span className="sec-n">{pad(1)}</span>
              Needs attention
              <span className="rule" />
            </h2>
            {sections.failing.map((r) => row(r, true))}
          </section>
        )}
        {showRunning && (
          <section className="group running">
            <h2>
              <span className="sec-n">{pad(showFailing ? 2 : 1)}</span>
              Running
              <span className="rule" />
            </h2>
            {sections.running.map((r) => row(r, true))}
          </section>
        )}
        {filter === "all" &&
          sections.groups.map((g, i) => (
            <section key={g.id} className={`group${g.muted ? " is-muted" : ""}`}>
              <h2>
                <span className="sec-n">{pad(firstRepoSection + i)}</span>
                <span className={`provider ${g.provider}`}>{g.provider === "github" ? "GH" : "BB"}</span>
                <span className="repo-name">{g.repo}</span>
                {editingClient === g.id ? (
                  <input
                    className="client-input"
                    autoFocus
                    list="client-names"
                    defaultValue={settings?.clients[g.id] ?? ""}
                    placeholder="Client"
                    aria-label={`Client for ${g.repo}`}
                    // Marked done so the blur that follows unmounting doesn't save a second time.
                    onKeyDown={(e) => {
                      const input = e.currentTarget;
                      if (e.key === "Enter") {
                        input.dataset.done = "1";
                        setClient(g.id, input.value);
                      } else if (e.key === "Escape") {
                        e.stopPropagation();
                        input.dataset.done = "1";
                        setEditingClient(null);
                      }
                    }}
                    onBlur={(e) => !e.currentTarget.dataset.done && setClient(g.id, e.currentTarget.value)}
                  />
                ) : (
                  settings?.clients[g.id] && <span className="client-chip">{settings.clients[g.id]}</span>
                )}
                {g.pinned && <span className="pinned-label">Pinned</span>}
                <span className="rule" />
                <span className="repo-tools">
                  <button
                    title="Set the client"
                    aria-label={`Set the client for ${g.repo}`}
                    onClick={() => setEditingClient(g.id)}
                  >
                    <IconTag />
                  </button>
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

      <datalist id="client-names">
        {[...new Set(Object.values(settings?.clients ?? {}))].map((c) => (
          <option key={c} value={c} />
        ))}
      </datalist>
      <footer className="hints" aria-hidden>
        {[
          ["↑↓", "select"],
          ["↵", "open"],
          ["⌘L", "log"],
          ["⌘R", "re-run"],
          ["⌘D", "watch"],
          ["esc", "close"],
        ].map(([key, what]) => (
          <span key={key}>
            <kbd>{key}</kbd>
            {what}
          </span>
        ))}
      </footer>
    </div>
  );
}
