import type { DispatcherTask } from "../views/Dispatcher";

interface Props {
  task: DispatcherTask;
  onClose: () => void;
}

export default function PayloadViewer({ task, onClose }: Props) {
  let pretty = task.task_payload;
  try {
    pretty = JSON.stringify(JSON.parse(task.task_payload), null, 2);
  } catch {
    /* keep raw */
  }

  return (
    <div style={overlay} onClick={onClose}>
      <div style={modal} onClick={(e) => e.stopPropagation()}>
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 12 }}>
          <h3 style={{ margin: 0, fontSize: 16 }}>
            📡 Task <code style={{ fontSize: 12 }}>{task.id}</code>
          </h3>
          <button type="button" onClick={onClose} style={closeBtn}>✕</button>
        </div>

        <div style={metaRow}>
          <Meta label="from" value={task.from_entity} />
          <Meta label="→ to" value={task.to_entity} />
          <Meta label="status" value={task.status} />
          {task.execution_time_ms != null && <Meta label="exec_ms" value={String(task.execution_time_ms)} />}
          <Meta label="created" value={task.created_at} />
        </div>

        <label style={{ fontSize: 12, color: "#666", fontWeight: 600 }}>Payload</label>
        <pre style={pre}>{pretty}</pre>

        <div style={{ display: "flex", justifyContent: "flex-end", gap: 8, marginTop: 12 }}>
          <button
            type="button"
            onClick={() => navigator.clipboard.writeText(pretty).catch(() => {})}
            style={primaryBtn}
          >
            Скопировать
          </button>
          <button type="button" onClick={onClose} style={secondaryBtn}>Закрыть</button>
        </div>
      </div>
    </div>
  );
}

function Meta({ label, value }: { label: string; value: string }) {
  return (
    <div style={{ fontSize: 12 }}>
      <div style={{ color: "#888", fontWeight: 600 }}>{label}</div>
      <div style={{ fontFamily: "ui-monospace, monospace", color: "#222" }}>{value}</div>
    </div>
  );
}

const overlay: React.CSSProperties = {
  position: "fixed", inset: 0, background: "rgba(0,0,0,0.5)",
  display: "flex", alignItems: "center", justifyContent: "center", zIndex: 1000,
};
const modal: React.CSSProperties = {
  background: "#fff", borderRadius: 8, padding: 24,
  width: "min(720px, 92vw)", maxHeight: "90vh", overflowY: "auto",
};
const metaRow: React.CSSProperties = {
  display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(140px, 1fr))",
  gap: 12, marginBottom: 16, padding: 12, background: "#fafafa", borderRadius: 6,
};
const pre: React.CSSProperties = {
  background: "#1a1a1a", color: "#9ef5a4", padding: 14, borderRadius: 6,
  fontSize: 12, fontFamily: "ui-monospace, monospace", overflowX: "auto",
  maxHeight: 400, marginTop: 6, whiteSpace: "pre-wrap", wordBreak: "break-all",
};
const closeBtn: React.CSSProperties = {
  background: "transparent", border: "none", fontSize: 18, cursor: "pointer", color: "#888",
};
const primaryBtn: React.CSSProperties = {
  padding: "8px 16px", background: "#1a1a1a", color: "#fff",
  border: "none", borderRadius: 4, cursor: "pointer", fontSize: 13, fontWeight: 600,
};
const secondaryBtn: React.CSSProperties = {
  padding: "8px 16px", background: "#fff", color: "#1a1a1a",
  border: "1px solid #ccc", borderRadius: 4, cursor: "pointer", fontSize: 13,
};
