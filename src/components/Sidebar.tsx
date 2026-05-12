import type { CSSProperties } from "react";

export type View = "home" | "ceo" | "vault" | "dispatcher" | "settings";

interface SidebarProps {
  current: View;
  onChange: (next: View) => void;
}

const ITEMS: Array<{ id: View; label: string; icon: string }> = [
  { id: "home", label: "Главная", icon: "🏠" },
  { id: "ceo", label: "Гендир (CEO)", icon: "💬" },
  { id: "vault", label: "Отдел СБ", icon: "🔐" },
  { id: "dispatcher", label: "Диспетчер", icon: "📡" },
  { id: "settings", label: "Настройки", icon: "⚙" },
];

const containerStyle: CSSProperties = {
  width: 220,
  flexShrink: 0,
  background: "#1a1a1a",
  color: "#e0e0e0",
  display: "flex",
  flexDirection: "column",
  padding: "20px 0",
  fontFamily: "system-ui, -apple-system, sans-serif",
  height: "100vh",
  overflowY: "auto",
};

const headerStyle: CSSProperties = {
  padding: "0 20px 20px",
  borderBottom: "1px solid #333",
  marginBottom: 12,
  flexShrink: 0,
};

const itemStyle = (active: boolean): CSSProperties => ({
  padding: "12px 20px",
  cursor: "pointer",
  background: active ? "#2a2a2a" : "transparent",
  borderLeft: active ? "3px solid #4caf50" : "3px solid transparent",
  display: "flex",
  alignItems: "center",
  gap: 12,
  fontSize: 14,
  color: active ? "#fff" : "#bbb",
  transition: "background 0.15s, color 0.15s",
});

export default function Sidebar({ current, onChange }: SidebarProps) {
  return (
    <nav style={containerStyle} aria-label="Главная навигация">
      <div style={headerStyle}>
        <div style={{ fontSize: 16, fontWeight: 700, color: "#fff" }}>MSPro-Ltd Corp</div>
        <div style={{ fontSize: 11, color: "#888", marginTop: 2 }}>v1.0.8 · Шаг 7.3</div>
      </div>
      {ITEMS.map((item) => (
        <button
          key={item.id}
          type="button"
          onClick={() => onChange(item.id)}
          style={{
            ...itemStyle(current === item.id),
            border: "none",
            textAlign: "left",
            width: "100%",
          }}
          aria-current={current === item.id ? "page" : undefined}
        >
          <span style={{ fontSize: 18 }}>{item.icon}</span>
          <span>{item.label}</span>
        </button>
      ))}
    </nav>
  );
}
