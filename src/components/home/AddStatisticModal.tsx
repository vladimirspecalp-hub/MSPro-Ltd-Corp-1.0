import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { PostHMT } from "../../types/hmt";

interface Props {
  postId: string;
  postTitle: string;
  metric?: string | null;
  onClose: () => void;
  onSaved: (hmt: PostHMT) => void;
}

export default function AddStatisticModal({
  postId,
  postTitle,
  metric,
  onClose,
  onSaved,
}: Props) {
  const [value, setValue] = useState<string>("");
  const [recordedAt, setRecordedAt] = useState<string>(""); // datetime-local
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const parsed = parseFloat(value);
  const canSubmit = Number.isFinite(parsed) && !submitting;

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    if (!canSubmit) return;
    setSubmitting(true);
    setError(null);
    try {
      const recorded = recordedAt
        ? new Date(recordedAt).toISOString().replace("T", " ").substring(0, 19)
        : null;
      const hmt = await invoke<PostHMT>("add_statistic_value", {
        input: {
          post_id: postId,
          value: parsed,
          recorded_at: recorded,
        },
      });
      onSaved(hmt);
    } catch (err) {
      setError(String(err));
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <div style={overlay} onClick={onClose} role="dialog" aria-modal="true">
      <div style={modal} onClick={(e) => e.stopPropagation()}>
        <h2 style={{ marginTop: 0, fontSize: 18 }}>📊 Добавить значение статистики</h2>
        <p style={{ margin: "4px 0 16px", color: "#666", fontSize: 13 }}>
          Пост: <strong>{postTitle}</strong>
          {metric && (
            <>
              {" · метрика: "}
              <code style={{ background: "#f5f5f5", padding: "1px 6px", borderRadius: 3 }}>
                {metric}
              </code>
            </>
          )}
        </p>

        <form onSubmit={submit}>
          <div style={field}>
            <label style={label} htmlFor="stat-value">Значение</label>
            <input
              id="stat-value"
              type="number"
              step="any"
              style={input}
              value={value}
              onChange={(e) => setValue(e.target.value)}
              placeholder="12.5"
              autoFocus
              required
            />
            <div style={help}>любое число (целое или дробное)</div>
          </div>

          <div style={field}>
            <label style={label} htmlFor="stat-time">Дата записи (опционально)</label>
            <input
              id="stat-time"
              type="datetime-local"
              style={input}
              value={recordedAt}
              onChange={(e) => setRecordedAt(e.target.value)}
            />
            <div style={help}>если пусто — будет «сейчас»</div>
          </div>

          {error && <div style={errBox}>{error}</div>}

          <div style={{ display: "flex", justifyContent: "flex-end", gap: 8, marginTop: 8 }}>
            <button type="button" onClick={onClose} style={cancelBtn}>
              Отмена
            </button>
            <button type="submit" disabled={!canSubmit} style={submitBtn(canSubmit)}>
              {submitting ? "Сохраняю…" : "Сохранить"}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}

const overlay: React.CSSProperties = {
  position: "fixed", inset: 0, background: "rgba(0,0,0,0.5)",
  display: "flex", alignItems: "center", justifyContent: "center", zIndex: 1000,
};
const modal: React.CSSProperties = {
  background: "#fff", borderRadius: 8, padding: 24,
  width: "min(480px, 92vw)", maxHeight: "90vh", overflowY: "auto",
};
const field: React.CSSProperties = { marginBottom: 14 };
const label: React.CSSProperties = {
  display: "block", fontSize: 13, fontWeight: 600, marginBottom: 6, color: "#333",
};
const input: React.CSSProperties = {
  display: "block", width: "100%", padding: "10px 12px", fontSize: 14,
  border: "1px solid #ccc", borderRadius: 4, fontFamily: "inherit", boxSizing: "border-box",
};
const help: React.CSSProperties = { fontSize: 11, color: "#888", marginTop: 4 };
const errBox: React.CSSProperties = {
  padding: 12, background: "#fee", border: "1px solid #c00", borderRadius: 4,
  fontSize: 13, marginBottom: 14, whiteSpace: "pre-wrap",
};
const cancelBtn: React.CSSProperties = {
  padding: "10px 20px", background: "#fff", color: "#333",
  border: "1px solid #ccc", borderRadius: 4, cursor: "pointer", fontSize: 14,
};
const submitBtn = (enabled: boolean): React.CSSProperties => ({
  padding: "10px 20px", background: enabled ? "#1565c0" : "#aaa", color: "#fff",
  border: "none", borderRadius: 4, cursor: enabled ? "pointer" : "not-allowed",
  fontSize: 14, fontWeight: 600,
});
