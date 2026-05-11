import {
  CONDITION_COLORS,
  CONDITION_EMOJI,
  CONDITION_LABELS_RU,
  type Condition,
} from "../../types/hmt";

interface Props {
  condition: Condition;
  size?: "sm" | "md";
}

export default function ConditionBadge({ condition, size = "sm" }: Props) {
  const c = CONDITION_COLORS[condition];
  const emoji = CONDITION_EMOJI[condition];
  const label = CONDITION_LABELS_RU[condition];

  const padding = size === "md" ? "4px 12px" : "2px 8px";
  const fontSize = size === "md" ? 13 : 11;

  return (
    <span
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: 4,
        padding,
        borderRadius: 12,
        background: c.bg,
        color: c.fg,
        fontSize,
        fontWeight: 600,
        whiteSpace: "nowrap",
      }}
      title={`Состояние: ${label}`}
    >
      <span aria-hidden style={{ fontSize: fontSize - 1 }}>
        {emoji}
      </span>
      {label}
    </span>
  );
}
