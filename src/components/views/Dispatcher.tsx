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
}

type Tab = "active" | "all" | "failed";

export default function Dispatcher() {
  const [tab, setTab] = useState<Tab>("active");
  const [tasks, setTasks] = useState<DispatcherTask[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState("");
  const [opened, setOpened] = useState<DispatcherTask | null>(null);

  async function refresh() {
    setLoading(true);
    setError(null);
    try {
      if (tab === "active") {
        const list = await invoke<DispatcherTask[]>("list_active_tasks");
        setTasks(list);
      } else {
        const list = await invoke<DispatcherTask[]>("list_recent_tasks", { limit: 500 });
        setTasks(tab === "failed" ? list.filter((t) => t.status === "failed") : list);
      }
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
      unlisten = await listen<DispatcherTask>("dispatcher-task-changed", (event) => {
        const t = event.payload;
        setTasks((prev) => {
          const matchesTab =
            tab === "all" ||
            (tab === "active" && (t.status === "in_progress" || t.status === "failed")) ||
            (tab === "failed" && t.status === "failed");
          const without = prev.filter((p) => p.id !== t.id);
          return matchesTab ? [t, ...without] : without;
        });
        setOpened((cur) => (cur && cur.id === t.id ? t : cur));
      });
    })();
    return () => {
      if (unlisten) unlisten();
    };
  }, [tab]);

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

      <div style={{ display: "flex", gap: 6, marginBottom: 16 }}>
        {(["active", "all", "failed"] as Tab[]).map((t) => (
          <button
            key={t}
            type="button"
            onClick={() => setTab(t)}
            style={{
              padding: "8px 16px",
              background: tab === t ? "#1a1a1a" : "#fff",
              color: tab === t ? "#fff" : "#1a1a1a",
              border: "1px solid " + (tab === t ? "#1a1a1a" : "#ccc"),
              borderRadius: 6,
              cursor: "pointer",
              fontSize: 13,
              fontWeight: 600,
            }}
          >
            {t === "active" ? "🟡 Активные" : t === "all" ? "📋 Все" : "❌ Failed"}
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
