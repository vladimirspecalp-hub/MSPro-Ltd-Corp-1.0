import { invoke } from "@tauri-apps/api/core";
import type { DispatcherTask } from "../views/Dispatcher";

interface Props {
  task: DispatcherTask;
  onOpen: (task: DispatcherTask) => void;
  onChanged: (task: DispatcherTask) => void;
}

const STATUS_STYLE: Record<string, { bg: string; fg: string; label: string }> = {
  in_progress: { bg: "#fff3cd", fg: "#856404", label: "⏳ in_progress" },
  completed: { bg: "#d4edda", fg: "#155724", label: "✅ completed" },
  failed: { bg: "#f8d7da", fg: "#721c24", label: "❌ failed" },
};

export default function TaskRow({ task, onOpen, onChanged }: Props) {
  const st = STATUS_STYLE[task.status] ?? STATUS_STYLE.in_progress;
  const preview = task.task_payload.length > 80
    ? task.task_payload.slice(0, 80) + "…"
    : task.task_payload;

  async function quickComplete(e: React.MouseEvent) {
    e.stopPropagation();
    try {
      const updated = await invoke<DispatcherTask>("complete_task", {
        taskId: task.id,
        executionTimeMs: null,
      });
      onChanged(updated);
    } catch (e) {
      alert(String(e));
    }
  }

  async function quickFail(e: React.MouseEvent) {
    e.stopPropagation();
    const reason = window.prompt("Причина провала:", "manual fail from UI");
    if (!reason) return;
    try {
      const updated = await invoke<DispatcherTask>("fail_task", {
        taskId: task.id,
        reason,
      });
      onChanged(updated);
    } catch (e) {
      alert(String(e));
    }
  }

  return (
    <tr style={{ borderTop: "1px solid #eee", cursor: "pointer" }} onClick={() => onOpen(task)}>
      <td style={{ ...td, fontFamily: "ui-monospace, monospace", fontSize: 12 }}>{task.from_entity}</td>
      <td style={{ ...td, textAlign: "center", color: "#999" }}>→</td>
      <td style={{ ...td, fontFamily: "ui-monospace, monospace", fontSize: 12 }}>{task.to_entity}</td>
      <td style={td}>
        <span
          style={{
            padding: "2px 10px", borderRadius: 12,
            background: st.bg, color: st.fg, fontSize: 11, fontWeight: 600,
          }}
        >
          {st.label}
        </span>
      </td>
      <td style={{ ...td, fontFamily: "ui-monospace, monospace", fontSize: 12, color: "#555", maxWidth: 320, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
        {preview}
      </td>
      <td style={{ ...td, color: "#999", fontSize: 12 }}>{task.execution_time_ms ?? "—"}</td>
      <td style={{ ...td, color: "#999", fontSize: 12, whiteSpace: "nowrap" }}>{relative(task.created_at)}</td>
      <td style={{ ...td, textAlign: "right", whiteSpace: "nowrap" }}>
        {task.status === "in_progress" && (
          <>
            <button type="button" onClick={quickComplete} style={actionBtn} title="Завершить">✅</button>
            <button type="button" onClick={quickFail} style={{ ...actionBtn, marginLeft: 4 }} title="Провалить">❌</button>
          </>
        )}
      </td>
    </tr>
  );
}

function relative(iso: string): string {
  const t = Date.parse(iso.endsWith("Z") || /[+\-]\d{2}:?\d{2}$/.test(iso) ? iso : iso + "Z");
  if (Number.isNaN(t)) return iso;
  const diff = (Date.now() - t) / 1000;
  if (diff < 60) return `${Math.floor(diff)} с назад`;
  if (diff < 3600) return `${Math.floor(diff / 60)} мин назад`;
  if (diff < 86400) return `${Math.floor(diff / 3600)} ч назад`;
  return `${Math.floor(diff / 86400)} д назад`;
}

const td: React.CSSProperties = { padding: "10px 14px", fontSize: 13 };
const actionBtn: React.CSSProperties = {
  background: "transparent", border: "1px solid #ddd", borderRadius: 4,
  cursor: "pointer", padding: "4px 8px", fontSize: 13,
};
