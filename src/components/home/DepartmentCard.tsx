import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import AddPostModal from "./AddPostModal";
import AddStatisticModal from "./AddStatisticModal";
import ConditionBadge from "./ConditionBadge";
import Sparkline from "./Sparkline";
import { CONDITION_COLORS, TREND_ARROW, type Condition, type PostHMT } from "../../types/hmt";

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
  const [hmtMap, setHmtMap] = useState<Record<string, PostHMT>>({});
  const [statPost, setStatPost] = useState<Post | null>(null);

  async function refresh() {
    setLoading(true);
    setError(null);
    try {
      const list = await invoke<Post[]>("list_posts_by_dept", { departmentId: dept.id });
      setPosts(list);
      // Подгружаем HMT-карточку для каждого поста параллельно.
      const hmtPairs = await Promise.all(
        list.map(async (p) => {
          try {
            const h = await invoke<PostHMT>("get_post_hmt", { input: { post_id: p.id } });
            return [p.id, h] as const;
          } catch {
            return null;
          }
        }),
      );
      const next: Record<string, PostHMT> = {};
      for (const pair of hmtPairs) {
        if (pair) next[pair[0]] = pair[1];
      }
      setHmtMap(next);
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

  // Подписка на live-обновления HMT — любые добавления статистик (UI/WS) обновят бейдж/spark.
  useEffect(() => {
    if (!open) return;
    let unlisten: UnlistenFn | null = null;
    (async () => {
      unlisten = await listen<PostHMT>("post-hmt-changed", (event) => {
        const h = event.payload;
        setHmtMap((prev) => ({ ...prev, [h.post_id]: h }));
      });
    })();
    return () => {
      if (unlisten) unlisten();
    };
  }, [open]);

  // Step 9: live-обновление списка постов после Гендир-CRUD (create / update / archive).
  // Emit-ится из src-tauri/src/commands/tool_calls.rs::execute_*_post.
  useEffect(() => {
    if (!open) return;
    let unlisten: UnlistenFn | null = null;
    (async () => {
      unlisten = await listen<{
        kind: "created" | "updated" | "archived";
        department_id?: string;
        old_department_id?: string;
      }>("posts-changed", (event) => {
        const p = event.payload;
        // Перерисовываем если событие касается нашего отделения (в том числе
        // если пост переезжает «оттуда» или «сюда»).
        if (
          !p.department_id ||
          p.department_id === dept.id ||
          p.old_department_id === dept.id
        ) {
          refresh();
        }
      });
    })();
    return () => {
      if (unlisten) unlisten();
    };
  }, [open, dept.id]);

  function onCreated(post: Post) {
    setPosts((prev) => [...prev, post]);
    setShowAddModal(false);
  }

  function onStatSaved(hmt: PostHMT) {
    setHmtMap((prev) => ({ ...prev, [hmt.post_id]: hmt }));
    setStatPost(null);
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
                  {(() => {
                    const h = hmtMap[p.id];
                    const cond: Condition = h?.condition ?? "NonExistence";
                    const color = CONDITION_COLORS[cond].fg;
                    const arrow = h?.trend_direction ? TREND_ARROW[h.trend_direction] : "";
                    return (
                      <div
                        style={{
                          display: "flex",
                          alignItems: "center",
                          gap: 8,
                          marginTop: 8,
                          flexWrap: "wrap",
                        }}
                      >
                        <ConditionBadge condition={cond} />
                        <Sparkline values={h?.sparkline_values ?? []} color={color} />
                        {h?.last_value != null && (
                          <span style={{ fontSize: 12, color: "#555", fontFamily: "ui-monospace, monospace" }}>
                            {h.last_value.toFixed(1)} {arrow}
                          </span>
                        )}
                        <button
                          type="button"
                          onClick={() => setStatPost(p)}
                          style={{
                            marginLeft: "auto",
                            padding: "4px 10px",
                            background: "#fff",
                            border: "1px solid #ccc",
                            borderRadius: 4,
                            cursor: "pointer",
                            fontSize: 12,
                          }}
                          title="Добавить значение статистики"
                        >
                          📊 +
                        </button>
                      </div>
                    );
                  })()}
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

      {statPost && (
        <AddStatisticModal
          postId={statPost.id}
          postTitle={statPost.title}
          metric={statPost.main_statistic_metric}
          onClose={() => setStatPost(null)}
          onSaved={onStatSaved}
        />
      )}
    </div>
  );
}
