import { useEffect, useMemo, useState } from "react";
import { api, errorText } from "./api";
import type { BuildTime, Run, Settings } from "./types";
import { repoId } from "./types";
import { formatSecs, isActive } from "./model";
import { IconBack, StatusIcon } from "./icons";

const NO_CLIENT = "No client";

// Build time is grouped by UTC month, matching how the backend stores it.
function month(offset: number) {
  const now = new Date();
  return new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth() + offset, 1)).toISOString().slice(0, 7);
}

interface RepoRow {
  id: string;
  name: string;
  failing: number;
  running: number;
  secs: number;
  runs: number;
}

interface Props {
  runs: Run[];
  settings: Settings | null;
  onBack: () => void;
}

export function ClientsView({ runs, settings, onBack }: Props) {
  const [offset, setOffset] = useState(0);
  const [time, setTime] = useState<BuildTime[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let stale = false;
    setTime(null);
    setError(null);
    api
      .buildTime(month(offset))
      .then((t) => !stale && setTime(t))
      .catch((e) => !stale && setError(errorText(e)));
    return () => {
      stale = true;
    };
  }, [offset]);

  const clients = useMemo(() => {
    const assigned = settings?.clients ?? {};
    const repos = new Map<string, RepoRow>();
    const row = (id: string) => {
      let r = repos.get(id);
      if (!r) {
        r = { id, name: id.replace(/^(github|bitbucket):/, ""), failing: 0, running: 0, secs: 0, runs: 0 };
        repos.set(id, r);
      }
      return r;
    };
    // Runs come newest first, so the first per workflow and branch is its current state.
    const seen = new Set<string>();
    for (const r of runs) {
      const id = repoId(r);
      if (!(id in assigned)) continue;
      const repo = row(id);
      if (isActive(r)) repo.running++;
      const workflow = `${id}|${r.title}|${r.branch}`;
      if (seen.has(workflow)) continue;
      seen.add(workflow);
      if (r.status === "failed") repo.failing++;
    }
    for (const t of time ?? []) Object.assign(row(t.repoId), { secs: t.secs, runs: t.runs });
    for (const id of Object.keys(assigned)) row(id);

    const byClient = new Map<string, RepoRow[]>();
    for (const r of repos.values()) {
      const name = assigned[r.id] ?? NO_CLIENT;
      byClient.set(name, [...(byClient.get(name) ?? []), r]);
    }
    return [...byClient.entries()]
      .map(([name, list]) => ({
        name,
        repos: list.sort((a, b) => b.secs - a.secs || a.name.localeCompare(b.name)),
        failing: list.reduce((n, r) => n + r.failing, 0),
        running: list.reduce((n, r) => n + r.running, 0),
        secs: list.reduce((n, r) => n + r.secs, 0),
        runs: list.reduce((n, r) => n + r.runs, 0),
      }))
      .sort((a, b) => Number(a.name === NO_CLIENT) - Number(b.name === NO_CLIENT) || b.secs - a.secs);
  }, [runs, settings, time]);

  const hasClients = Object.keys(settings?.clients ?? {}).length > 0;
  return (
    <div className="app">
      <header className="bar">
        <button className="icon" title="Back (esc)" aria-label="Back" onClick={onBack}>
          <IconBack />
        </button>
        <strong>Clients</strong>
        <div className="spacer" />
        <div className="segmented" role="group" aria-label="Month">
          <button className={offset === -1 ? "on" : ""} onClick={() => setOffset(-1)}>
            Last month
          </button>
          <button className={offset === 0 ? "on" : ""} onClick={() => setOffset(0)}>
            This month
          </button>
        </div>
      </header>
      {error && <div className="banner error">{error}</div>}
      <main className="list clients">
        {!hasClients && (
          <p className="empty">
            Hover a repo heading in the list and click the tag to give it a client. Build time is totalled here per
            client and month.
          </p>
        )}
        {clients.map((c) => (
          <section key={c.name} className="client">
            <h2>
              <StatusIcon status={c.failing ? "failed" : c.running ? "running" : "success"} />
              <span className="client-name">{c.name}</span>
              <span className="client-total" title={`${c.runs} finished runs`}>
                {time ? formatSecs(c.secs) : error ? "–" : "…"}
              </span>
            </h2>
            {c.repos.map((r) => (
              <div key={r.id} className="client-repo">
                <span className="client-repo-name">{r.name}</span>
                {r.failing > 0 && <span className="chip failed">{r.failing} failing</span>}
                {r.running > 0 && <span className="chip running">{r.running} running</span>}
                <span className="client-time">{r.runs ? `${formatSecs(r.secs)} · ${r.runs} runs` : "No runs"}</span>
              </div>
            ))}
          </section>
        ))}
        <p className="hint client-note">
          Bitbucket times are billed build minutes. GitHub times are wall-clock time per run, which can differ from
          billed minutes when jobs run in parallel.
        </p>
      </main>
    </div>
  );
}
