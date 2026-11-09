import { useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api, errorText } from "./api";
import { IconBack } from "./icons";
import type { ProviderStatus } from "./types";

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
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.settings().then((s) => {
      setNotifyAll(s.notifyAll);
      setBbEmail(s.bitbucketEmail ?? "");
      setMuted(s.muted);
      // On first run, offer it ticked; it only takes effect on "Get started".
      setLaunchAtLogin(s.launchAtLogin === null ? null : welcome ? true : s.launchAtLogin);
    });
  }, []);

  const run = async (id: string, fn: () => Promise<void>) => {
    setBusy(id);
    setError(null);
    try {
      await fn();
      setGhToken("");
      setBbToken("");
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
          <button className="icon" title="Back (esc)" aria-label="Back" onClick={onBack}>
            <IconBack />
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
