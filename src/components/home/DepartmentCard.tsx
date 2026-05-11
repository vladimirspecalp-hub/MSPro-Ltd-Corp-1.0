import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import AddPostModal from "./AddPostModal";

export interface Department {
  id: string;
  dept_number: number;
  name: string;
  description: string | null;
}

export interface Post {
  id: string;
  department_id: string;
  slug: string;
  title: string;
  central_product: string;
  main_statistic_metric: string | null;
  status: string;
  created_at: string;
}

interface Props {
  dept: Department;
  defaultOpen?: boolean;
}

const cardStyle: React.CSSProperties = {
  background: "#fff",
  borderRadius: 8,
  border: "1px solid #ddd",
  overflow: "hidden",
  display: "flex",
  flexDirection: "column",
};

const headerStyle: React.CSSProperties = {
  padding: "14px 16px",
  display: "flex",
  alignItems: "center",
  gap: 12,
  cursor: "pointer",
  background: "#f5f5f5",
  borderBottom: "1px solid #eee",
};

const numberStyle: React.CSSProperties = {
  fontSize: 22,
  fontWeight: 700,
  width: 32,
  textAlign: "center",
  color: "#1a1a1a",
};

export default function DepartmentCard({ dept, defaultOpen = false }: Props) {
  const [open, setOpen] = useState(defaultOpen);
  const [posts, setPosts] = useState<Post[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [showAddModal, setShowAddModal] = useState(false);

  async function refresh() {
    setLoading(true);
    setError(null);
    try {
      const list = await invoke<Post[]>("list_posts_by_dept", { departmentId: dept.id });
      setPosts(list);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    if (open) {
      refresh();
    }
  }, [open]);

  function onCreated(post: Post) {
    setPosts((prev) => [...prev, post]);
    setShowAddModal(false);
  }

  return (
    <div style={cardStyle}>
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        style={{
          ...headerStyle,
          border: "none",
          width: "100%",
          textAlign: "left",
        }}
        aria-expanded={open}
      >
        <span style={numberStyle}>{dept.dept_number}</span>
        <div style={{ flex: 1 }}>
          <div style={{ fontWeight: 600, fontSize: 15 }}>{dept.name}</div>
          {dept.description && (
            <div style={{ fontSize: 12, color: "#666", marginTop: 2 }}>{dept.description}</div>
          )}
        </div>
        <span style={{ fontSize: 12, color: "#888" }}>{open ? "▲" : "▼"}</span>
      </button>

      {open && (
        <div style={{ padding: "12px 16px" }}>
          {loading && <p style={{ color: "#999", fontSize: 13 }}>Загружаю посты…</p>}
          {error && (
            <p style={{ color: "#c00", fontSize: 13, whiteSpace: "pre-wrap" }}>Ошибка: {error}</p>
          )}
          {!loading && !error && posts.length === 0 && (
            <p style={{ color: "#999", fontSize: 13, fontStyle: "italic" }}>Постов пока нет</p>
          )}
          {posts.length > 0 && (
            <ul style={{ listStyle: "none", padding: 0, margin: "0 0 12px" }}>
              {posts.map((p) => (
                <li
                  key={p.id}
                  style={{
                    padding: "8px 12px",
                    marginBottom: 6,
                    background: "#fafafa",
                    borderLeft: "3px solid #4caf50",
                    borderRadius: 4,
                    fontSize: 13,
                  }}
                >
                  <div style={{ display: "flex", gap: 8, alignItems: "baseline" }}>
                    <code
                      style={{
                        background: "#e8e8e8",
                        padding: "1px 6px",
                        borderRadius: 3,
                        fontSize: 11,
                      }}
                    >
                      {p.slug}
                    </code>
                    <strong>{p.title}</strong>
                  </div>
                  <div style={{ fontSize: 12, color: "#555", marginTop: 4 }}>
                    ЦКП: {p.central_product}
                  </div>
                  {p.main_statistic_metric && (
                    <div style={{ fontSize: 11, color: "#888", marginTop: 2 }}>
                      📊 {p.main_statistic_metric}
                    </div>
                  )}
                </li>
              ))}
            </ul>
          )}
          <button
            type="button"
            onClick={() => setShowAddModal(true)}
            style={{
              padding: "8px 14px",
              background: "#1a1a1a",
              color: "#fff",
              border: "none",
              borderRadius: 4,
              cursor: "pointer",
              fontSize: 13,
              fontWeight: 600,
            }}
          >
            + Добавить пост
          </button>
        </div>
      )}

      {showAddModal && (
        <AddPostModal
          departmentId={dept.id}
          departmentName={dept.name}
          onClose={() => setShowAddModal(false)}
          onCreated={onCreated}
        />
      )}
    </div>
  );
}
