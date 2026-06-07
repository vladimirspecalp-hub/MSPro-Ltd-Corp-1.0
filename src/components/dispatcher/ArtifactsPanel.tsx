// v1.0.22 Phase 11C — Artifacts list per task with approve/reject buttons.

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import FileActions from "../common/FileActions";

interface Artifact {
  id: string;
  task_id: string;
  rel_path: string;
  mime_type: string | null;
  size_bytes: number | null;
  created_by: string;
  created_at: string;
  approved_at: string | null;
  rejected_at: string | null;
  reject_reason: string | null;
}

interface Props {
  taskId: string;
}

export default function ArtifactsPanel({ taskId }: Props) {
  const [list, setList] = useState<Artifact[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function refresh() {
    try {
      const data = await invoke<Artifact[]>("list_task_artifacts", { taskId });
      setList(data);
    } catch (e) {
      setError(String(e));
    }
  }

  useEffect(() => {
    refresh();
  }, [taskId]);

  async function onApprove(id: string) {
    setBusy(true);
    setError(null);
    try {
      await invoke("approve_artifact", { artifactId: id });
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  async function onReject(id: string) {
    const reason = window.prompt("Причина отклонения:");
    if (!reason || !reason.trim()) return;
    setBusy(true);
    setError(null);
    try {
      await invoke("reject_artifact", {
        input: { artifact_id: id, reject_reason: reason.trim() },
      });
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  if (error) {
    return <div style={{ fontSize: 12, color: "#c00" }}>artifacts error: {error}</div>;
  }
  if (list == null) {
    return <div style={{ fontSize: 12, color: "#888" }}>загружаю артефакты…</div>;
  }
  if (list.length === 0) {
    return (
      <div style={{ fontSize: 12, color: "#888", fontStyle: "italic" }}>
        Артефактов пока нет. Когда исполнитель (пост-агент) положит файл в Outbox/, он
        появится здесь для утверждения.
      </div>
    );
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
      {list.map((a) => {
        const status = a.approved_at
          ? "✅ Утверждён"
          : a.rejected_at
            ? "❌ Отклонён"
            : "👁 Ожидает решения";
        const statusColor = a.approved_at
          ? "#1f6f3b"
          : a.rejected_at
            ? "#a51b1b"
            : "#a06800";
        return (
          <div
            key={a.id}
            style={{
              padding: 10,
              background: "#fafafa",
              border: "1px solid #ddd",
              borderRadius: 4,
              fontSize: 12,
            }}
          >
            <div style={{ display: "flex", alignItems: "baseline", gap: 8 }}>
              <code style={{ fontSize: 11, fontWeight: 600 }}>{a.rel_path}</code>
              <span style={{ color: "#888" }}>
                {a.size_bytes != null ? `${a.size_bytes} bytes` : ""}
              </span>
              <span style={{ marginLeft: "auto", color: statusColor, fontWeight: 600 }}>
                {status}
              </span>
            </div>
            <div style={{ fontSize: 11, color: "#666", marginTop: 2 }}>
              by <strong>{a.created_by}</strong> · {a.created_at}
              {a.mime_type ? ` · ${a.mime_type}` : ""}
            </div>
            {a.reject_reason && (
              <div
                style={{
                  marginTop: 4,
                  padding: 4,
                  background: "#fff2f0",
                  fontSize: 11,
                  color: "#a51b1b",
                  borderRadius: 3,
                }}
              >
                Причина: {a.reject_reason}
              </div>
            )}
            <div style={{ display: "flex", gap: 6, marginTop: 6 }}>
              <FileActions artifactId={a.id} onError={setError} />
              {!a.approved_at && (
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => onApprove(a.id)}
                  style={btnStyle("#e6f5ea")}
                >
                  ✅ Утвердить
                </button>
              )}
              {!a.rejected_at && (
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => onReject(a.id)}
                  style={btnStyle("#fff2f0")}
                >
                  ❌ Отклонить
                </button>
              )}
            </div>
          </div>
        );
      })}
    </div>
  );
}

function btnStyle(bg: string): React.CSSProperties {
  return {
    padding: "4px 10px",
    background: bg,
    border: "1px solid #ccc",
    borderRadius: 3,
    cursor: "pointer",
    fontSize: 11,
  };
}
