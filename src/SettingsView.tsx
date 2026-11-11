import { useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api, errorText } from "./api";
import { IconBack } from "./icons";
import type { ProviderStatus } from "./types";
import type { Theme } from "./theme";
import { loadTheme, saveTheme } from "./theme";

const GITHUB_TOKEN_URL = "https://github.com/settings/tokens/new?scopes=repo&description=Pipescope";
const ATLASSIAN_TOKEN_URL = "https://id.atlassian.com/manage-profile/security/api-tokens";

function describe(s: ProviderStatus | undefined) {
  if (!s || s.state === "notConfigured") return "Not connected";
  if (s.state === "connecting") return "Connecting…";
  if (s.state === "error" || s.state === "offline") return s.message ?? "Error";
  const via = s.source ? ` via ${s.source}` : "";
  return `Connected as ${s.account}${via} · ${s.repos} repos`;
}

interface Props {
  statuses: ProviderStatus[];
  onBack: () => void;
  // First run: an intro and a "Get started" button instead of a back arrow.
  welcome?: boolean;
}

export function SettingsView({ statuses, onBack, welcome }: Props) {
  const gh = statuses.find((s) => s.provider === "github");
  const bb = statuses.find((s) => s.provider === "bitbucket");
  const [ghToken, setGhToken] = useState("");
  const [bbEmail, setBbEmail] = useState("");
  const [bbToken, setBbToken] = useState("");
  const [notifyAll, setNotifyAll] = useState(false);
  const [muted, setMuted] = useState<string[]>([]);
  const [launchAtLogin, setLaunchAtLogin] = useState<boolean | null>(null);
  const [branches, setBranches] = useState("");
  const [quietHours, setQuietHours] = useState("");
  const [rulesSaved, setRulesSaved] = useState(true);
  const [autoRerun, setAutoRerun] = useState(false);
  const [ignored, setIgnored] = useState("");
  const [ignoredSaved, setIgnoredSaved] = useState(true);
  const [codeFolders, setCodeFolders] = useState("");
  const [defaultFolders, setDefaultFolders] = useState("");
  const [foldersSaved, setFoldersSaved] = useState(true);
  const [checkouts, setCheckouts] = useState<number | null>(null);
  const [theme, setTheme] = useState<Theme>(loadTheme);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.settings().then((s) => {
      setNotifyAll(s.notifyAll);
      setBbEmail(s.bitbucketEmail ?? "");
      setMuted(s.muted);
      setBranches(s.notifyBranches);
      setQuietHours(s.quietHours);
      setAutoRerun(s.autoRerunFlaky);
      setIgnored(s.ignoredRepos ?? "");
      setCodeFolders(s.codeFolders);
      setDefaultFolders(s.defaultCodeFolders);
      // On first run, offer it ticked; it only takes effect on "Get started".
      setLaunchAtLogin(s.launchAtLogin === null ? null : welcome ? true : s.launchAtLogin);
    });
  }, []);

  const countCheckouts = () =>
    api
      .checkouts()
      .then((c) => setCheckouts(Object.keys(c).length))
      .catch(() => setCheckouts(null));
  useEffect(() => {
    countCheckouts();
  }, []);

  const run = async (id: string, fn: () => Promise<void>) => {
    setBusy(id);
    setError(null);
    try {
      await fn();
      // Only a token save should clear the token fields; other saves leave half-typed ones alone.
      if (id === "gh") setGhToken("");
      if (id === "bb") setBbToken("");
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="app">
      <header className="bar">
        {!welcome && (
          <button className="back" title="Back (esc)" onClick={onBack}>
            <IconBack /> Runs
          </button>
        )}
        <strong>{welcome ? "Welcome to Pipescope" : "Settings"}</strong>
      </header>
      <main className="settings">
        {welcome && (
          <p className="intro">
            Pipescope watches your GitHub Actions and Bitbucket Pipelines from the menu bar. Connect at least one
            account, then press <kbd>⌥⌘P</kbd> any time to open it.
          </p>
        )}
        {error && <div className="banner error">{error}</div>}

        <section>
          <h3>GitHub</h3>
          <p className={`status ${gh?.state ?? ""}`}>{describe(gh)}</p>
          {gh?.warnings.map((w) => (
            <p key={w} className="status warning">
              {w}
            </p>
          ))}
          <input
            type="password"
            placeholder="Personal access token (classic)"
            value={ghToken}
            onChange={(e) => setGhToken(e.target.value)}
          />
          <p className="hint">
            Needs the <code>repo</code> scope. Leave blank to use your <code>gh</code> CLI login. For SSO orgs, authorise the token for each org.{" "}
            <a onClick={() => openUrl(GITHUB_TOKEN_URL)}>Create a token</a>
          </p>
          <div className="row">
            <button disabled={!!busy || !ghToken} onClick={() => run("gh", () => api.setGithubToken(ghToken))}>
              {busy === "gh" ? "Checking…" : "Save token"}
            </button>
            {gh?.source === "Keychain" && (
              <button className="secondary" disabled={!!busy} onClick={() => run("gh", () => api.setGithubToken(""))}>
                Use gh CLI instead
              </button>
            )}
          </div>
        </section>

        <section>
          <h3>Bitbucket Cloud</h3>
          <p className={`status ${bb?.state ?? ""}`}>{describe(bb)}</p>
          <input type="email" placeholder="Atlassian account email" value={bbEmail} onChange={(e) => setBbEmail(e.target.value)} />
          <input type="password" placeholder="API token" value={bbToken} onChange={(e) => setBbToken(e.target.value)} />
          <p className="hint">
            Create a scoped API token for Bitbucket with <code>read:user</code>, <code>read:workspace</code>,{" "}
            <code>read:repository</code>,{" "}
            <code>read:pipeline</code> and <code>write:pipeline</code>.{" "}
            <a onClick={() => openUrl(ATLASSIAN_TOKEN_URL)}>Create a token</a>
          </p>
          <div className="row">
            <button
              disabled={!!busy || !bbEmail || !bbToken}
              onClick={() => run("bb", () => api.setBitbucketAuth(bbEmail, bbToken))}
            >
              {busy === "bb" ? "Checking…" : "Save"}
            </button>
            {bb && bb.state !== "notConfigured" && (
              <button
                className="secondary"
                disabled={!!busy}
                onClick={() => run("bb", async () => {
                  await api.setBitbucketAuth("", "");
                  setBbEmail("");
                })}
              >
                Disconnect
              </button>
            )}
          </div>
        </section>

        <section>
          <h3>Notifications</h3>
          <label className="check">
            <input
              type="checkbox"
              checked={notifyAll}
              onChange={(e) => {
                setNotifyAll(e.target.checked);
                api.setNotifyAll(e.target.checked).catch((err) => setError(errorText(err)));
              }}
            />
            Notify and colour the tray icon for everyone's runs, not just mine
          </label>
          <label className="field">
            <span>Only notify for these branches</span>
            <input
              type="text"
              placeholder="All branches, or e.g. main, release/*"
              value={branches}
              onChange={(e) => {
                setBranches(e.target.value);
                setRulesSaved(false);
              }}
            />
          </label>
          <label className="field">
            <span>Quiet hours</span>
            <input
              type="text"
              placeholder="e.g. 22:00-07:00"
              value={quietHours}
              onChange={(e) => {
                setQuietHours(e.target.value);
                setRulesSaved(false);
              }}
            />
          </label>
          <p className="hint">
            Runs you're watching always notify when they finish, whatever these say. A run counts as yours when you
            started it or it's your commit.
          </p>
          <div className="row">
            <button
              disabled={!!busy || rulesSaved}
              onClick={() =>
                run("rules", async () => {
                  await api.setNotifyRules(branches, quietHours);
                  setRulesSaved(true);
                })
              }
            >
              {rulesSaved ? "Saved" : "Save rules"}
            </button>
          </div>
        </section>

        <section>
          <h3>Flaky workflows</h3>
          <label className="check">
            <input
              type="checkbox"
              checked={autoRerun}
              onChange={(e) => {
                const on = e.target.checked;
                setAutoRerun(on);
                api.setAutoRerun(on).catch((err) => {
                  setAutoRerun(!on);
                  setError(errorText(err));
                });
              }}
            />
            Re-run a failed workflow once automatically if it has recently passed on retry
          </label>
          <p className="hint">
            Only for runs you'd be notified about. You're only told about the failure if the re-run fails too. Uses
            build minutes.
          </p>
        </section>
        <section>
          <h3>Appearance</h3>
          <div className="choice" role="group" aria-label="Appearance">
            {(["system", "light", "dark"] as const).map((t) => (
              <button
                key={t}
                className={theme === t ? "on" : ""}
                aria-pressed={theme === t}
                onClick={() => {
                  setTheme(t);
                  saveTheme(t);
                }}
              >
                {{ system: "Match macOS", light: "Light", dark: "Dark" }[t]}
              </button>
            ))}
          </div>
        </section>
        <section>
          <h3>Startup</h3>
          <label className="check">
            <input
              type="checkbox"
              checked={!!launchAtLogin}
              disabled={launchAtLogin === null}
              onChange={(e) => {
                const on = e.target.checked;
                setLaunchAtLogin(on);
                if (!welcome) {
                  api.setLaunchAtLogin(on).catch((err) => {
                    setLaunchAtLogin(!on);
                    setError(errorText(err));
                  });
                }
              }}
            />
            Open Pipescope when you log in
          </label>
          {launchAtLogin === null && <p className="hint">Only available in the built app, not in development.</p>}
        </section>
        <section>
          <h3>Ignored repos</h3>
          <label className="field">
            <span>Don't follow repos matching</span>
            <textarea
              rows={3}
              placeholder={"e.g. acme/legacy-*, old-client/*"}
              value={ignored}
              onChange={(e) => {
                setIgnored(e.target.value);
                setIgnoredSaved(false);
              }}
            />
          </label>
          <p className="hint">
            One <code>owner/repo</code> per line or comma, with <code>*</code> for anything. A name on its own, like{" "}
            <code>acme</code>, means all of that owner's repos. Matching repos are never
            fetched, so they use no API calls, and their runs are removed. Build time already counted is kept. To hide a
            repo but keep watching it, mute it instead.
          </p>
          <div className="row">
            <button
              disabled={!!busy || ignoredSaved}
              onClick={() =>
                run("ignored", async () => {
                  await api.setIgnoredRepos(ignored);
                  setIgnoredSaved(true);
                })
              }
            >
              {ignoredSaved ? "Saved" : "Save"}
            </button>
          </div>
        </section>
        <section>
          <h3>Local checkouts</h3>
          <label className="field">
            <span>Look for clones in</span>
            <textarea
              rows={3}
              placeholder={defaultFolders.split("\n").join(", ")}
              value={codeFolders}
              onChange={(e) => {
                setCodeFolders(e.target.value);
                setFoldersSaved(false);
              }}
            />
          </label>
          <p className="hint">
            One folder per line or comma. Pipescope looks four folders deep for git clones and matches them to repos by
            their remotes, so a copied failure can say where the code is. Blank searches the folders shown.
            {checkouts !== null && ` Found ${checkouts} ${checkouts === 1 ? "repo" : "repos"}.`}
          </p>
          <div className="row">
            <button
              disabled={!!busy || foldersSaved}
              onClick={() =>
                run("folders", async () => {
                  await api.setCodeFolders(codeFolders);
                  setFoldersSaved(true);
                  await countCheckouts();
                })
              }
            >
              {foldersSaved ? "Saved" : "Save"}
            </button>
          </div>
        </section>
        {muted.length > 0 && (
          <section>
            <h3>Muted repos</h3>
            <p className="hint">Hidden from the list and the tray, and never notify.</p>
            {muted.map((id) => (
              <div key={id} className="muted-row">
                <span>{id.replace(/^(github|bitbucket):/, "")}</span>
                <button
                  className="text"
                  onClick={() =>
                    run("mute", async () => {
                      await api.setRepoFlag(id, "muted", false);
                      setMuted((m) => m.filter((x) => x !== id));
                    })
                  }
                >
                  Unmute
                </button>
              </div>
            ))}
          </section>
        )}

        {welcome && (
          <div className="row welcome-done">
            <button
              disabled={!statuses.some((s) => s.state === "connected" || s.state === "connecting")}
              onClick={() =>
                run("start", async () => {
                  if (launchAtLogin) await api.setLaunchAtLogin(true);
                  onBack();
                })
              }
            >
              Get started
            </button>
          </div>
        )}
      </main>
    </div>
  );
}
