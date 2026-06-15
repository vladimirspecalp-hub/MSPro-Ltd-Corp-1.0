import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import TaskRow from "../dispatcher/TaskRow";
import PayloadViewer from "../dispatcher/PayloadViewer";

export interface DispatcherTask {
  id: string;
  from_entity: string;
  to_entity: string;
  task_payload: string;
  status: string;
  execution_time_ms: number | null;
  created_at: string;
  // v1.0.22 Phase 11C — Hub-and-Spoke audit
  parent_task_id?: string | null;
  completed_at?: string | null;
  attempts_count?: number | null;
  hop_kind?: string | null;
  routed_by_model?: string | null;
  refined_prompt?: string | null;
  outbox_path?: string | null;
}

export interface DispatcherDecision {
  id: string;
  source_task_id: string;
  result_task_id: string | null;
  decision_kind: string;
  reasoning: string | null;
  model_used: string;
  routing_complexity: string | null;
  elapsed_ms: number | null;
  created_at: string;
}

type Tab = "inbox" | "processing" | "awaiting" | "completed" | "failed" | "cancelled" | "all";

export default function Dispatcher() {
  const [tab, setTab] = useState<Tab>("inbox");
  const [tasks, setTasks] = useState<DispatcherTask[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState("");
  const [opened, setOpened] = useState<DispatcherTask | null>(null);

  async function refresh() {
    setLoading(true);
    setError(null);
    try {
      // Все вкладки v1.0.22 фильтруют clientside из последних 500
      const list = await invoke<DispatcherTask[]>("list_recent_tasks", { limit: 500 });
      const filtered = list.filter((t) => {
        switch (tab) {
          case "inbox":
            return t.hop_kind === "raw_request" && t.status === "in_progress";
          case "processing":
            return t.status === "in_progress" && t.hop_kind !== "raw_request";
          case "awaiting":
            return t.status === "in_progress" && (t.outbox_path != null && t.outbox_path !== "");
          case "completed":
            return t.status === "completed";
          case "failed":
            return t.status === "failed";
          case "cancelled":
            return t.status === "cancelled";
          case "all":
            return true;
        }
      });
      setTasks(filtered);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    refresh();
  }, [tab]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    (async () => {
      unlisten = await listen<DispatcherTask>("dispatcher-task-changed", (_event) => {
        // Простой refresh — фильтрация по вкладкам клиентская, проще перетянуть.
        refresh();
      });
    })();
    return () => {
      if (unlisten) unlisten();
    };
  }, [tab]); // eslint-disable-line react-hooks/exhaustive-deps

  const filtered = useMemo(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return tasks;
    return tasks.filter(
      (t) =>
        t.from_entity.toLowerCase().includes(q) ||
        t.to_entity.toLowerCase().includes(q) ||
        t.task_payload.toLowerCase().includes(q),
    );
  }, [tasks, filter]);

  return (
    <div style={{ padding: "32px 48px", overflowY: "auto", maxWidth: 1400 }}>
      <header style={{ borderBottom: "2px solid #1a1a1a", paddingBottom: 16, marginBottom: 24 }}>
        <h1 style={{ margin: 0, fontSize: 28 }}>📡 Диспетчер</h1>
        <p style={{ margin: "4px 0 0", color: "#666", fontSize: 14 }}>
          Шина задач между агентами. Любой пост, n8n workflow или внешний бот может прислать сюда
          задачу через WebSocket RPC <code>dispatcher/submit</code>.
        </p>
      </header>

      <div style={{ display: "flex", gap: 6, marginBottom: 16, flexWrap: "wrap" }}>
        {([
          { id: "inbox" as Tab, label: "📥 Inbox" },
          { id: "processing" as Tab, label: "⚙️ Processing" },
          { id: "awaiting" as Tab, label: "👁 Awaiting" },
          { id: "completed" as Tab, label: "✅ Completed" },
          { id: "failed" as Tab, label: "❌ Failed" },
          { id: "cancelled" as Tab, label: "⊘ Cancelled" },
          { id: "all" as Tab, label: "📋 Все" },
        ]).map((t) => (
          <button
            key={t.id}
            type="button"
            onClick={() => setTab(t.id)}
            style={{
              padding: "8px 16px",
              background: tab === t.id ? "#1a1a1a" : "#fff",
              color: tab === t.id ? "#fff" : "#1a1a1a",
              border: "1px solid " + (tab === t.id ? "#1a1a1a" : "#ccc"),
              borderRadius: 6,
              cursor: "pointer",
              fontSize: 13,
              fontWeight: 600,
            }}
          >
            {t.label}
          </button>
        ))}
        <input
          type="text"
          placeholder="Фильтр по from / to / payload…"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          style={{
            flex: 1, marginLeft: 12, padding: "8px 12px",
            border: "1px solid #ccc", borderRadius: 6, fontSize: 13,
          }}
        />
        <button
          type="button"
          onClick={refresh}
          style={{
            padding: "8px 16px", background: "#fff", color: "#1a1a1a",
            border: "1px solid #ccc", borderRadius: 6, cursor: "pointer", fontSize: 13,
          }}
        >
          ↻
        </button>
        <span style={{ color: "#888", fontSize: 13, alignSelf: "center", minWidth: 60, textAlign: "right" }}>
          {filtered.length} зад.
        </span>
      </div>

      {error && (
        <div style={{ padding: 12, background: "#fee", border: "1px solid #c00", borderRadius: 4, fontSize: 13, marginBottom: 16, whiteSpace: "pre-wrap" }}>
          {error}
        </div>
      )}

      {loading && <p style={{ color: "#999" }}>Загружаю…</p>}

      {!loading && filtered.length === 0 && !error && (
        <div style={{ padding: 40, background: "#fafafa", border: "1px dashed #ccc", borderRadius: 8, textAlign: "center", color: "#888" }}>
          <p style={{ fontSize: 16, margin: 0 }}>
            🪹 Очередь пустая. Жду первую задачу через <code>dispatcher/submit</code>.
          </p>
        </div>
      )}

      {filtered.length > 0 && (
        <table style={{ width: "100%", borderCollapse: "collapse", background: "#fff", borderRadius: 6, overflow: "hidden", boxShadow: "0 1px 3px rgba(0,0,0,0.05)" }}>
          <thead>
            <tr style={{ background: "#f5f5f5", textAlign: "left" }}>
              <th style={th}>From</th>
              <th style={th}></th>
              <th style={th}>To</th>
              <th style={th}>Status</th>
              <th style={th}>Payload</th>
              <th style={th}>ms</th>
              <th style={th}>Когда</th>
              <th style={{ ...th, textAlign: "right" }}>Действия</th>
            </tr>
          </thead>
          <tbody>
            {filtered.map((t) => (
              <TaskRow
                key={t.id}
                task={t}
                onOpen={setOpened}
                onChanged={(upd) => setTasks((prev) => prev.map((p) => (p.id === upd.id ? upd : p)))}
              />
            ))}
          </tbody>
        </table>
      )}

      {opened && <PayloadViewer task={opened} onClose={() => setOpened(null)} />}
    </div>
  );
}

const th: React.CSSProperties = { padding: "10px 14px", fontWeight: 600, fontSize: 12, color: "#555" };
