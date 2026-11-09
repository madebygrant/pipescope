import type { RunStatus } from "./types";

const base = { width: 14, height: 14, viewBox: "0 0 14 14", "aria-hidden": true } as const;

export function StatusIcon({ status }: { status: RunStatus }) {
  switch (status) {
    case "success":
      return (
        <svg {...base} className="status-icon success">
          <circle cx="7" cy="7" r="7" />
          <path d="M3.8 7.2l2.1 2.1 4.3-4.6" fill="none" stroke="#fff" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      );
    case "failed":
      return (
        <svg {...base} className="status-icon failed">
          <circle cx="7" cy="7" r="7" />
          <path d="M4.6 4.6l4.8 4.8M9.4 4.6l-4.8 4.8" stroke="#fff" strokeWidth="1.8" strokeLinecap="round" />
        </svg>
      );
    case "running":
      return (
        <svg {...base} className="status-icon running spin">
          <circle cx="7" cy="7" r="5.5" fill="none" strokeWidth="2" opacity="0.3" />
          <path d="M7 1.5a5.5 5.5 0 0 1 5.5 5.5" fill="none" strokeWidth="2" strokeLinecap="round" />
        </svg>
      );
    case "queued":
      return (
        <svg {...base} className="status-icon queued">
          <circle cx="7" cy="7" r="5.5" fill="none" strokeWidth="2" strokeDasharray="2.6 2" />
        </svg>
      );
    case "waiting":
      return (
        <svg {...base} className="status-icon waiting">
          <circle cx="7" cy="7" r="5.5" fill="none" strokeWidth="1.8" />
          <path d="M5.6 4.8v4.4M8.4 4.8v4.4" strokeWidth="1.6" strokeLinecap="round" />
        </svg>
      );
    default:
      return (
        <svg {...base} className="status-icon neutral">
          <circle cx="7" cy="7" r="7" />
          <path d="M4.5 7h5" stroke="#fff" strokeWidth="1.8" strokeLinecap="round" />
        </svg>
      );
  }
}

const ui = { width: 16, height: 16, viewBox: "0 0 16 16", fill: "none", stroke: "currentColor", strokeWidth: 1.6, strokeLinecap: "round", strokeLinejoin: "round", "aria-hidden": true } as const;

export const IconRefresh = () => (
  <svg {...ui}>
    <path d="M13 8a5 5 0 1 1-1.5-3.6" />
    <path d="M13 2.5v3h-3" />
  </svg>
);
export const IconGear = () => (
  <svg {...ui}>
    <path d="M2.5 4.5h11M2.5 11.5h11" />
    <circle cx="10" cy="4.5" r="1.8" fill="var(--panel)" />
    <circle cx="6" cy="11.5" r="1.8" fill="var(--panel)" />
  </svg>
);
export const IconBack = () => (
  <svg {...ui}>
    <path d="M10 3L5 8l5 5" />
  </svg>
);
export const IconExternal = () => (
  <svg {...ui} width={12} height={12}>
    <path d="M9 2.5h4.5V7M13.5 2.5L7.5 8.5M11.5 9.5v3.5h-9v-9H6" />
  </svg>
);
export const IconSearch = () => (
  <svg {...ui} width={14} height={14}>
    <circle cx="7" cy="7" r="4.5" />
    <path d="M10.5 10.5l3 3" />
  </svg>
);
export const IconPin = ({ filled }: { filled?: boolean }) => (
  <svg {...ui} width={13} height={13}>
    <path d="M6 2.5h4l-.6 4 2.1 2.2H4.5l2.1-2.2z" fill={filled ? "currentColor" : "none"} />
    <path d="M8 8.7v4.8" />
  </svg>
);
export const IconMute = ({ muted }: { muted?: boolean }) => (
  <svg {...ui} width={13} height={13}>
    <path d="M4 10.5h8l-1-1.6V6.5a3 3 0 0 0-6 0v2.4z" />
    <path d="M6.8 12.5a1.3 1.3 0 0 0 2.4 0" />
    {muted && <path d="M2.5 2.5l11 11" />}
  </svg>
);
export const IconCopy = () => (
  <svg {...ui} width={13} height={13}>
    <rect x="5" y="5" width="8.5" height="8.5" rx="1.5" />
    <path d="M10.5 5V3.5A1 1 0 0 0 9.5 2.5h-6a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1H5" />
  </svg>
);
export const IconChevron = ({ dir }: { dir: "up" | "down" | "right" }) => (
  <svg {...ui} width={12} height={12}>
    <path d={dir === "up" ? "M4 10l4-4 4 4" : dir === "down" ? "M4 6l4 4 4-4" : "M6 4l4 4-4 4"} />
  </svg>
);
export const IconWrap = () => (
  <svg {...ui} width={13} height={13}>
    <path d="M2.5 4h11M2.5 8h9a2 2 0 0 1 0 4H8M9.5 10.5L8 12l1.5 1.5M2.5 12h3" />
  </svg>
);
export const IconExpand = () => (
  <svg {...ui} width={14} height={14}>
    <path d="M9.5 2.5h4v4M13.5 2.5L9 7M6.5 13.5h-4v-4M2.5 13.5L7 9" />
  </svg>
);
export const IconWarning = () => (
  <svg {...ui} width={14} height={14}>
    <path d="M8 2.5l6 10.5H2z" />
    <path d="M8 6.8v2.6M8 11.4v.1" />
  </svg>
);
