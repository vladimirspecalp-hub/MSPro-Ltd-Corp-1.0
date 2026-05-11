import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Post } from "./DepartmentCard";

interface Props {
  departmentId: string;
  departmentName: string;
  onClose: () => void;
  onCreated: (post: Post) => void;
}

const SLUG_RE = /^[a-z0-9](?:[a-z0-9-]{0,38}[a-z0-9])?$/;

const overlayStyle: React.CSSProperties = {
  position: "fixed",
  inset: 0,
  background: "rgba(0,0,0,0.5)",
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  zIndex: 1000,
};

const modalStyle: React.CSSProperties = {
  background: "#fff",
  borderRadius: 8,
  padding: 24,
  width: "min(560px, 92vw)",
  maxHeight: "90vh",
  overflowY: "auto",
  boxShadow: "0 20px 60px rgba(0,0,0,0.3)",
};

const fieldStyle: React.CSSProperties = {
  display: "block",
  width: "100%",
  padding: "10px 12px",
  fontSize: 14,
  border: "1px solid #ccc",
  borderRadius: 4,
  fontFamily: "inherit",
  boxSizing: "border-box",
};

const labelStyle: React.CSSProperties = {
  display: "block",
  fontSize: 13,
  fontWeight: 600,
  marginBottom: 6,
  color: "#333",
};

const helpStyle: React.CSSProperties = {
  fontSize: 11,
  color: "#888",
  marginTop: 4,
};

export default function AddPostModal({
  departmentId,
  departmentName,
  onClose,
  onCreated,
}: Props) {
  const [slug, setSlug] = useState("");
  const [title, setTitle] = useState("");
  const [centralProduct, setCentralProduct] = useState("");
  const [metric, setMetric] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const slugValid = SLUG_RE.test(slug);
  const titleValid = title.trim().length >= 2 && title.trim().length <= 200;
  const cpValid = centralProduct.trim().length >= 5 && centralProduct.trim().length <= 500;
  const canSubmit = slugValid && titleValid && cpValid && !submitting;

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!canSubmit) return;
    setSubmitting(true);
    setError(null);
    try {
      const post = await invoke<Post>("create_post", {
        input: {
          department_id: departmentId,
          slug: slug.trim(),
          title: title.trim(),
          central_product: centralProduct.trim(),
          main_statistic_metric: metric.trim() || null,
        },
      });
      onCreated(post);
    } catch (e) {
      setError(String(e));
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <div style={overlayStyle} onClick={onClose} role="dialog" aria-modal="true">
      <div style={modalStyle} onClick={(e) => e.stopPropagation()}>
        <h2 style={{ marginTop: 0, fontSize: 20 }}>
          + Новый пост
          <span style={{ fontSize: 13, color: "#888", marginLeft: 8 }}>в «{departmentName}»</span>
        </h2>

        <form onSubmit={handleSubmit}>
          <div style={{ marginBottom: 14 }}>
            <label style={labelStyle} htmlFor="post-slug">
              Slug
            </label>
            <input
              id="post-slug"
              style={{
                ...fieldStyle,
                fontFamily: "ui-monospace, monospace",
                borderColor: slug && !slugValid ? "#c00" : "#ccc",
              }}
              value={slug}
              onChange={(e) => setSlug(e.target.value.toLowerCase())}
              placeholder="frontend"
              maxLength={40}
              autoFocus
            />
            <div style={helpStyle}>
              латиница a-z, цифры, дефис; 2–40 символов; без пробелов
              {slug && !slugValid && <span style={{ color: "#c00" }}> · недопустимый формат</span>}
            </div>
          </div>

          <div style={{ marginBottom: 14 }}>
            <label style={labelStyle} htmlFor="post-title">
              Название
            </label>
            <input
              id="post-title"
              style={fieldStyle}
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              placeholder="Frontend разработчик"
              maxLength={200}
            />
            <div style={helpStyle}>
              как пост называется в живой речи · {title.trim().length} / 200
            </div>
          </div>

          <div style={{ marginBottom: 14 }}>
            <label style={labelStyle} htmlFor="post-cp">
              ЦКП (Центральный Производимый Продукт)
            </label>
            <textarea
              id="post-cp"
              style={{ ...fieldStyle, minHeight: 70, resize: "vertical" }}
              value={centralProduct}
              onChange={(e) => setCentralProduct(e.target.value)}
              placeholder="Working features, deployed без регрессий"
              maxLength={500}
            />
            <div style={helpStyle}>
              конкретный измеримый результат поста · {centralProduct.trim().length} / 500
            </div>
          </div>

          <div style={{ marginBottom: 14 }}>
            <label style={labelStyle} htmlFor="post-metric">
              Главная метрика <span style={{ color: "#888", fontWeight: 400 }}>(опционально)</span>
            </label>
            <input
              id="post-metric"
              style={{ ...fieldStyle, fontFamily: "ui-monospace, monospace" }}
              value={metric}
              onChange={(e) => setMetric(e.target.value)}
              placeholder="pull_requests_merged_per_week"
              maxLength={100}
            />
            <div style={helpStyle}>один основной показатель для отслеживания</div>
          </div>

          {error && (
            <div
              style={{
                padding: 12,
                background: "#fee",
                border: "1px solid #c00",
                borderRadius: 4,
                fontSize: 13,
                marginBottom: 14,
                whiteSpace: "pre-wrap",
              }}
            >
              {error}
            </div>
          )}

          <div style={{ display: "flex", gap: 8, justifyContent: "flex-end" }}>
            <button
              type="button"
              onClick={onClose}
              style={{
                padding: "10px 20px",
                background: "#fff",
                color: "#333",
                border: "1px solid #ccc",
                borderRadius: 4,
                cursor: "pointer",
                fontSize: 14,
              }}
            >
              Отмена
            </button>
            <button
              type="submit"
              disabled={!canSubmit}
              style={{
                padding: "10px 20px",
                background: canSubmit ? "#4caf50" : "#aaa",
                color: "#fff",
                border: "none",
                borderRadius: 4,
                cursor: canSubmit ? "pointer" : "not-allowed",
                fontSize: 14,
                fontWeight: 600,
              }}
            >
              {submitting ? "Создаю…" : "Создать пост"}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
