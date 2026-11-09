export type Provider = "github" | "bitbucket";
export type RunStatus = "queued" | "running" | "waiting" | "success" | "failed" | "cancelled" | "skipped";

export interface Run {
  key: string;
  provider: Provider;
  repo: string;
  id: string;
  number: number;
  title: string;
  subtitle: string | null;
  branch: string | null;
  commit: string | null;
  status: RunStatus;
  actor: string | null;
  mine: boolean;
  createdAt: string;
  startedAt: string | null;
  updatedAt: string;
  webUrl: string;
}

export interface RateInfo {
  remaining: number;
  limit: number;
  resetSecs: number | null;
}

export interface ProviderStatus {
  provider: Provider;
  state: "notConfigured" | "connecting" | "connected" | "offline" | "error";
  account: string | null;
  message: string | null;
  repos: number;
  source: string | null;
  lastPoll: string | null;
  rate: RateInfo | null;
  warnings: string[];
}

export interface Settings {
  notifyAll: boolean;
  bitbucketEmail: string | null;
  pinned: string[];
  muted: string[];
  onboarded: boolean;
}

export interface LogStep {
  id: string;
  name: string;
  status: RunStatus;
}

export interface LogTail {
  stepId: string;
  step: string;
  steps: LogStep[];
  text: string;
  webUrl: string;
}

export const repoId = (r: { provider: Provider; repo: string }) => `${r.provider}:${r.repo}`;
