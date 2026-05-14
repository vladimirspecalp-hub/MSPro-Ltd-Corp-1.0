// Шаг 10 — статус Claude CLI + Qwen local под переключателем brain mode.
// Авто-poll каждые 30 сек (не блокирует ввод).

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { CheckCircle2, AlertTriangle } from "lucide-react";

type ClaudeCliStatus =
  | { kind: "available"; version: string; path: string }
  | { kind: "not_found"; configured_path: string; error: string };

type QwenStatus =
  | { kind: "available"; endpoint: string; model_count: number }
  | { kind: "unreachable"; endpoint: string; error: string };

interface Props {
  onClaudeReady?: (ready: boolean) => void;
  onQwenReady?: (ready: boolean) => void;
}

export default function BrainStatusBadges({ onClaudeReady, onQwenReady }: Props) {
  const [claude, setClaude] = useState<ClaudeCliStatus | null>(null);
  const [qwen, setQwen] = useState<QwenStatus | null>(null);

  useEffect(() => {
    let alive = true;

    async function poll() {
      try {
        const c = await invoke<ClaudeCliStatus>("detect_claude_cli");
        if (alive) {
          setClaude(c);
          onClaudeReady?.(c.kind === "available");
        }
      } catch {
        if (alive) onClaudeReady?.(false);
      }
      try {
        const q = await invoke<QwenStatus>("detect_qwen");
        if (alive) {
          setQwen(q);
          onQwenReady?.(q.kind === "available");
        }
      } catch {
        if (alive) onQwenReady?.(false);
      }
    }

    poll();
    const t = setInterval(poll, 30_000);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, [onClaudeReady, onQwenReady]);

  return (
    <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
      <Badge
        label="Claude CLI"
        ok={claude?.kind === "available"}
        detail={
          claude?.kind === "available"
            ? `v${claude.version.split("\n")[0] || "?"}`
            : claude?.error?.slice(0, 80) ?? "проверка…"
        }
      />
      <Badge
        label="Qwen 3 local"
        ok={qwen?.kind === "available"}
        detail={
          qwen?.kind === "available"
            ? `${qwen.model_count} моделей`
            : qwen?.error?.slice(0, 80) ?? "проверка…"
        }
      />
    </div>
  );
}

function Badge({ label, ok, detail }: { label: string; ok: boolean; detail: string }) {
  const colors = ok
    ? { fg: "#1b5e20", bg: "#e8f5e9", border: "#4caf50" }
    : { fg: "#7a0000", bg: "#fee", border: "#c00" };
  const Icon = ok ? CheckCircle2 : AlertTriangle;
  return (
    <div
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: 8,
        padding: "6px 10px",
        background: colors.bg,
        border: `1px solid ${colors.border}`,
        color: colors.fg,
        borderRadius: 6,
        fontSize: 12,
        lineHeight: 1.2,
      }}
      title={detail}
    >
      <Icon size={14} />
      <strong>{label}</strong>
      <span style={{ opacity: 0.75 }}>· {detail}</span>
    </div>
  );
}
