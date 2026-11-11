import { invoke } from "@tauri-apps/api/core";
import type { Baseline, BuildTime, LogTail, PendingDeployment, ProviderStatus, Run, Settings } from "./types";

export const api = {
  runs: () => invoke<Run[]>("get_runs"),
  run: (key: string) => invoke<Run | null>("get_run", { key }),
  status: () => invoke<ProviderStatus[]>("get_status"),
  settings: () => invoke<Settings>("get_settings"),
  setNotifyAll: (value: boolean) => invoke<void>("set_notify_all", { value }),
  setLaunchAtLogin: (on: boolean) => invoke<void>("set_launch_at_login", { on }),
  setGithubToken: (token: string) => invoke<void>("set_github_token", { token }),
  setBitbucketAuth: (email: string, token: string) => invoke<void>("set_bitbucket_auth", { email, token }),
  refresh: () => invoke<void>("refresh"),
  rerun: (key: string, failedOnly: boolean) => invoke<void>("rerun", { key, failedOnly }),
  cancel: (key: string) => invoke<void>("cancel", { key }),
  log: (key: string, step?: string) => invoke<LogTail>("get_log", { key, step: step ?? null }),
  openLogWindow: (key: string, step?: string) => invoke<void>("open_log_window", { key, step: step ?? null }),
  setRepoFlag: (repoId: string, flag: "pinned" | "muted", on: boolean) =>
    invoke<void>("set_repo_flag", { repoId, flag, on }),
  setOnboarded: () => invoke<void>("set_onboarded"),
  setNotifyRules: (branches: string, quietHours: string) => invoke<void>("set_notify_rules", { branches, quietHours }),
  setAutoRerun: (on: boolean) => invoke<void>("set_auto_rerun", { on }),
  setIgnoredRepos: (patterns: string) => invoke<void>("set_ignored_repos", { patterns }),
  watch: (key: string, on: boolean) => invoke<void>("watch_run", { key, on }),
  setRepoClient: (repoId: string, client: string | null) => invoke<void>("set_repo_client", { repoId, client }),
  buildTime: (month?: string) => invoke<BuildTime[]>("get_build_time", { month: month ?? null }),
  pendingDeployments: (key: string) => invoke<PendingDeployment[]>("get_pending_deployments", { key }),
  reviewDeployments: (key: string, approve: boolean) => invoke<void>("review_deployments", { key, approve }),
  manualStepUrl: (key: string) => invoke<string>("manual_step_url", { key }),
  baselineLog: (key: string, step: string) => invoke<Baseline | null>("get_baseline_log", { key, step }),
};

export const errorText = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : String(e));

export const label = (p: string) => (p === "github" ? "GitHub" : "Bitbucket");
