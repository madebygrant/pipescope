import { invoke } from "@tauri-apps/api/core";
import type { LogTail, ProviderStatus, Run, Settings } from "./types";

export const api = {
  runs: () => invoke<Run[]>("get_runs"),
  status: () => invoke<ProviderStatus[]>("get_status"),
  settings: () => invoke<Settings>("get_settings"),
  setNotifyAll: (value: boolean) => invoke<void>("set_notify_all", { value }),
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
};

export const errorText = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : String(e));

export const label = (p: string) => (p === "github" ? "GitHub" : "Bitbucket");
