export type Theme = "system" | "light" | "dark";

const KEY = "theme";
const systemDark = window.matchMedia("(prefers-color-scheme: dark)");

export function loadTheme(): Theme {
  try {
    const t = localStorage.getItem(KEY);
    return t === "light" || t === "dark" ? t : "system";
  } catch {
    return "system";
  }
}

// "system" is resolved here rather than in CSS so the dark palette is only written once.
function applyTheme(theme: Theme) {
  const dark = theme === "dark" || (theme === "system" && systemDark.matches);
  document.documentElement.dataset.theme = dark ? "dark" : "light";
}

export function saveTheme(theme: Theme) {
  try {
    localStorage.setItem(KEY, theme);
  } catch {}
  applyTheme(theme);
}

// The popover and log windows share storage, so a change in one restyles the other.
export function followTheme() {
  applyTheme(loadTheme());
  systemDark.addEventListener("change", () => applyTheme(loadTheme()));
  window.addEventListener("storage", (e) => {
    if (e.key === KEY) applyTheme(loadTheme());
  });
}
