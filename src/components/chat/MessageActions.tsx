// Hover-action bar для сообщений Гендира в чате.
// Появляется при наведении мыши (CSS :hover на .msg-row из App.css).

import { Brain, Trophy } from "lucide-react";
import type { VaultKind } from "./VaultSaveModal";

interface Props {
  onPick: (kind: VaultKind) => void;
}

export default function MessageActions({ onPick }: Props) {
  return (
    <div
      style={{
        display: "flex",
        gap: 6,
        alignItems: "center",
      }}
      aria-label="Сохранить сообщение в память Гендира"
    >
      <ActionButton
        icon={<Brain size={14} />}
        label="В Паттерны"
        accent="#1565c0"
        onClick={() => onPick("pattern")}
      />
      <ActionButton
        icon={<Trophy size={14} />}
        label="В Победы"
        accent="#2e7d32"
        onClick={() => onPick("win")}
      />
    </div>
  );
}

function ActionButton({
  icon,
  label,
  accent,
  onClick,
}: {
  icon: React.ReactNode;
  label: string;
  accent: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={label}
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: 5,
        padding: "4px 10px",
        background: "#fff",
        color: accent,
        border: `1px solid ${accent}40`,
        borderRadius: 12,
        cursor: "pointer",
        fontSize: 11,
        fontWeight: 600,
        boxShadow: "0 1px 2px rgba(0,0,0,0.05)",
        transition: "background 0.12s",
      }}
      onMouseEnter={(e) => {
        (e.currentTarget as HTMLButtonElement).style.background = accent + "10";
      }}
      onMouseLeave={(e) => {
        (e.currentTarget as HTMLButtonElement).style.background = "#fff";
      }}
    >
      {icon}
      <span>{label}</span>
    </button>
  );
}
