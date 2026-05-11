// HMT-engine — общие типы для React-стороны.
// Зеркалит src-tauri/src/commands/hmt.rs::PostHMT.

export type Condition =
  | "NonExistence"
  | "Danger"
  | "Emergency"
  | "Normal"
  | "Affluence"
  | "Power";

export interface PostHMT {
  post_id: string;
  last_value: number | null;
  trend_direction: "up" | "down" | "flat" | null;
  condition: Condition;
  condition_ru: string;
  sparkline_values: number[];
  last_assigned_at: string | null;
}

export const CONDITION_LABELS_RU: Record<Condition, string> = {
  NonExistence: "Не-существование",
  Danger: "Опасность",
  Emergency: "ЧП",
  Normal: "Норма",
  Affluence: "Изобилие",
  Power: "Власть",
};

export const CONDITION_EMOJI: Record<Condition, string> = {
  NonExistence: "⚪",
  Danger: "🔴",
  Emergency: "🟠",
  Normal: "🟢",
  Affluence: "🔵",
  Power: "🟣",
};

export const CONDITION_COLORS: Record<Condition, { fg: string; bg: string }> = {
  NonExistence: { fg: "#616161", bg: "#f5f5f5" },
  Danger: { fg: "#c62828", bg: "#ffebee" },
  Emergency: { fg: "#ef6c00", bg: "#fff3e0" },
  Normal: { fg: "#2e7d32", bg: "#e8f5e9" },
  Affluence: { fg: "#1565c0", bg: "#e3f2fd" },
  Power: { fg: "#6a1b9a", bg: "#f3e5f5" },
};

export const TREND_ARROW: Record<NonNullable<PostHMT["trend_direction"]>, string> = {
  up: "↑",
  down: "↓",
  flat: "→",
};
