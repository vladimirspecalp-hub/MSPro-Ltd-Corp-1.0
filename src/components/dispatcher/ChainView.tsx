// v1.0.22 Phase 11C — Audit chain view.
//
// Сматывает цепочку hop'ов снизу-вверх: текущая задача → её parent → ...
// → корень (raw_request от Гендира). Также показывает решения Диспетчера
// (dispatcher_decisions) — какая модель приняла решение, что переписала.

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { DispatcherTask, DispatcherDecision } from "../views/Dispatcher";

interface Props {
  taskId: string;
}

const HOP_LABELS: Record<string, string> = {
  raw_request: "🟡 Сырой запрос",
  refined: "✨ Переписан Диспетчером",
  subtask: "🧩 Subtask (декомпозиция)",
  retry: "🔁 Retry",
  clarification: "❓ Запрос уточнения",
};

export default function ChainView({ taskId }: Props) {
  const [chain, setChain] = useState<DispatcherTask[] | null>(null);
  const [decisions, setDecisions] = useState<DispatcherDecision[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [c, d] = await Promise.all([
          invoke<DispatcherTask[]>("get_task_chain", { taskId }),
          invoke<DispatcherDecision[]>("list_decisions_for_task", { taskId }),
        ]);
        if (cancelled) return;
        setChain(c);
        setDecisions(d);
      } catch (e) {
        if (!cancelled) setError(String(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [taskId]);

  if (error) {
    return <div style={{ fontSize: 12, color: "#c00" }}>chain error: {error}</div>;
  }
  if (chain == null) {
    return <div style={{ fontSize: 12, color: "#888" }}>загружаю цепочку…</div>;
  }
  if (chain.length <= 1) {
    return (
      <div style={{ fontSize: 12, color: "#888", fontStyle: "italic" }}>
        Это корневая задача — родителей нет.
      </div>
    );
  }

  // chain отсортирован [current, parent, grandparent, ..., root].
  // Рендерим в обратном порядке (root → current) для интуитивного чтения.
  const ordered = [...chain].reverse();

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
      {ordered.map((t, idx) => {
        const isCurrent = t.id === taskId;
        const hopLabel = t.hop_kind ? HOP_LABELS[t.hop_kind] ?? t.hop_kind : "—";
        const decision = decisions.find((d) => d.result_task_id === t.id);
        return (
          <div
            key={t.id}
            style={{
              padding: "8px 12px",
              background: isCurrent ? "#fff7e6" : "#fafafa",
              border: "1px solid " + (isCurrent ? "#f5b73b" : "#ddd"),
              borderRadius: 4,
              fontSize: 12,
            }}
          >
            <div style={{ display: "flex", gap: 8, alignItems: "baseline" }}>
              <span style={{ color: "#666", fontFamily: "monospace" }}>
                #{idx + 1}
              </span>
              <strong>{hopLabel}</strong>
              <code style={{ fontSize: 10, color: "#555" }}>{t.from_entity}</code>
              <span style={{ color: "#888" }}>→</span>
              <code style={{ fontSize: 10, color: "#555" }}>{t.to_entity}</code>
              <span style={{ color: "#888", marginLeft: "auto", fontSize: 10 }}>
                {t.created_at}
              </span>
            </div>
            {t.refined_prompt && (
              <div
                style={{
                  marginTop: 4,
                  padding: 6,
                  background: "#fff",
                  border: "1px solid #eee",
                  borderRadius: 3,
                  fontSize: 11,
                  whiteSpace: "pre-wrap",
                  maxHeight: 120,
                  overflow: "auto",
                }}
              >
                {t.refined_prompt}
              </div>
            )}
            {decision && (
              <div
                style={{
                  marginTop: 4,
                  fontSize: 11,
                  color: "#555",
                  fontStyle: "italic",
                }}
              >
                🤖 Решение Диспетчера: <strong>{decision.decision_kind}</strong>{" "}
                via <code>{decision.model_used}</code>
                {decision.routing_complexity && ` (${decision.routing_complexity})`}
                {decision.elapsed_ms != null && ` · ${decision.elapsed_ms}ms`}
                {decision.reasoning && (
                  <div style={{ marginTop: 2, color: "#444" }}>{decision.reasoning}</div>
                )}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}
