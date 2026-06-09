// Этап 1 (Заход 1) — Конструктор оргструктуры (вкладка «Оргсхема»).
//
// Две панели: слева визуальное дерево (Гендир → Отделения → Отделы → Агенты)
// + операции; справа — карточка-редактор агента (заглушка, полноценно — Заход 2).
//
// ГРАНИЦА Захода 1: операции работают ТОЛЬКО с БД (бэкенд org_chart.rs).
// Диск не трогаем. Гендир — фиксированный read-only узел (особый случай).

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface AgentNode {
  id: string;
  department_id: string;
  name: string;
  slug: string;
  role_label: string;
  status: string;
  folder_path: string | null;
  sort_order: number;
}
interface DepartmentNode {
  id: string;
  name: string;
  description: string | null;
  sort_order: number;
  agents: AgentNode[];
}
interface DivisionNode {
  id: string;
  name: string;
  description: string | null;
  sort_order: number;
  departments: DepartmentNode[];
}
interface OrgTree {
  divisions: DivisionNode[];
}

type MoveState =
  | { kind: "department"; id: string }
  | { kind: "agent"; id: string }
  | null;

const STATUS_META: Record<string, { label: string; color: string }> = {
  active: { label: "активен", color: "#1f6f3b" },
  paused: { label: "пауза", color: "#a06800" },
  off: { label: "выкл", color: "#a51b1b" },
};

export default function OrgStructure() {
  const [tree, setTree] = useState<OrgTree | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<AgentNode | null>(null);
  const [move, setMove] = useState<MoveState>(null);

  async function refresh() {
    try {
      const data = await invoke<OrgTree>("list_org_tree");
      setTree(data);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }

  useEffect(() => {
    refresh();
  }, []);

  // Обёртка: вызвать команду → обновить дерево, ошибку показать в баннере.
  async function run(cmd: string, args: Record<string, unknown>) {
    try {
      await invoke(cmd, args);
      setError(null);
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  // ----- Операции -----
  function addDivision() {
    const name = window.prompt("Название отделения:");
    if (name && name.trim()) run("create_division", { name: name.trim(), description: null });
  }
  function renameDivision(d: DivisionNode) {
    const name = window.prompt("Новое название отделения:", d.name);
    if (name && name.trim()) run("rename_division", { id: d.id, name: name.trim(), description: d.description });
  }
  function deleteDivision(d: DivisionNode) {
    if (window.confirm(`Удалить отделение «${d.name}» со всеми отделами и агентами (только из дерева, папки на диске не трогаются)?`))
      run("delete_division", { id: d.id });
  }
  function addDepartment(d: DivisionNode) {
    const name = window.prompt(`Название отдела в «${d.name}»:`);
    if (name && name.trim()) run("create_department", { divisionId: d.id, name: name.trim(), description: null });
  }
  function renameDepartment(dep: DepartmentNode) {
    const name = window.prompt("Новое название отдела:", dep.name);
    if (name && name.trim()) run("rename_department", { id: dep.id, name: name.trim(), description: dep.description });
  }
  function deleteDepartment(dep: DepartmentNode) {
    if (window.confirm(`Удалить отдел «${dep.name}» с агентами (только из дерева)?`))
      run("delete_department", { id: dep.id });
  }
  function addAgent(dep: DepartmentNode) {
    const name = window.prompt(`Имя агента в «${dep.name}»:`);
    if (name && name.trim()) run("create_agent", { departmentId: dep.id, name: name.trim(), roleLabel: "member" });
  }
  function renameAgent(a: AgentNode) {
    const name = window.prompt("Новое имя агента:", a.name);
    if (name && name.trim()) run("rename_agent", { id: a.id, name: name.trim() });
  }
  function deleteAgent(a: AgentNode) {
    if (window.confirm(`Удалить агента «${a.name}» из дерева? (папка на диске НЕ удаляется)`)) {
      run("delete_agent", { id: a.id });
      if (selected?.id === a.id) setSelected(null);
    }
  }

  function doMoveDepartment(depId: string, newDivisionId: string) {
    setMove(null);
    if (newDivisionId) run("move_department", { id: depId, newDivisionId });
  }
  function doMoveAgent(agentId: string, newDepartmentId: string) {
    setMove(null);
    if (newDepartmentId) run("move_agent", { id: agentId, newDepartmentId });
  }

  // Все отделы (для перемещения агента) с подписью «Отделение / Отдел».
  const allDepartments: Array<{ id: string; label: string }> =
    tree?.divisions.flatMap((d) =>
      d.departments.map((dep) => ({ id: dep.id, label: `${d.name} / ${dep.name}` })),
    ) ?? [];

  return (
    <div style={{ display: "flex", flex: 1, minHeight: 0, height: "100%" }}>
      {/* ЛЕВАЯ ПАНЕЛЬ — дерево + операции */}
      <div style={{ flex: "1 1 60%", overflowY: "auto", padding: "24px 28px", borderRight: "1px solid #ddd" }}>
        <header style={{ display: "flex", alignItems: "center", gap: 12, marginBottom: 16 }}>
          <h1 style={{ margin: 0, fontSize: 24 }}>🏢 Оргсхема</h1>
          <button type="button" onClick={addDivision} style={primaryBtn}>+ Отделение</button>
        </header>
        <p style={{ margin: "0 0 16px", color: "#888", fontSize: 12 }}>
          Конструктор оргструктуры (Этап 1, скелет). Операции работают с базой; папки на диске — следующий этап.
        </p>

        {error && (
          <div style={{ padding: 10, background: "#fff2f0", color: "#a51b1b", borderRadius: 4, fontSize: 13, marginBottom: 12 }}>
            {error}
          </div>
        )}

        {/* Гендир — фиксированный read-only узел над отделениями */}
        <div style={{ ...nodeBox, background: "#1a1a2e", color: "#fff", borderColor: "#1a1a2e" }}>
          <span style={{ fontSize: 16 }}>👑</span>
          <strong>Гендир (CEO)</strong>
          <span style={{ fontSize: 11, color: "#bbb", marginLeft: "auto" }}>над отделениями · не редактируется здесь</span>
        </div>

        {tree == null ? (
          <div style={{ color: "#888", fontSize: 13, marginTop: 12 }}>загружаю структуру…</div>
        ) : tree.divisions.length === 0 ? (
          <div style={{ color: "#888", fontSize: 13, fontStyle: "italic", marginTop: 12 }}>
            Отделений пока нет. Нажми «+ Отделение», чтобы начать строить структуру.
          </div>
        ) : (
          tree.divisions.map((d) => (
            <div key={d.id} style={{ marginTop: 12, marginLeft: 8 }}>
              <div style={{ ...nodeBox, background: "#f0eef5", borderColor: "#cfc8de" }}>
                <span style={{ fontSize: 15 }}>🏛</span>
                <strong>{d.name}</strong>
                <span style={rowActions}>
                  <button type="button" style={iconBtn} title="Переименовать" onClick={() => renameDivision(d)}>✏</button>
                  <button type="button" style={iconBtn} title="Добавить отдел" onClick={() => addDepartment(d)}>+ отдел</button>
                  <button type="button" style={iconBtn} title="Удалить" onClick={() => deleteDivision(d)}>🗑</button>
                </span>
              </div>

              {d.departments.map((dep) => (
                <div key={dep.id} style={{ marginLeft: 22, marginTop: 6 }}>
                  <div style={{ ...nodeBox, background: "#eef3f5", borderColor: "#cdd9de" }}>
                    <span style={{ fontSize: 14 }}>📁</span>
                    <span>{dep.name}</span>
                    <span style={rowActions}>
                      <button type="button" style={iconBtn} title="Переименовать" onClick={() => renameDepartment(dep)}>✏</button>
                      <button type="button" style={iconBtn} title="Добавить агента" onClick={() => addAgent(dep)}>+ агент</button>
                      <button type="button" style={iconBtn} title="Переместить в другое отделение" onClick={() => setMove({ kind: "department", id: dep.id })}>↗</button>
                      <button type="button" style={iconBtn} title="Удалить" onClick={() => deleteDepartment(dep)}>🗑</button>
                    </span>
                  </div>

                  {move?.kind === "department" && move.id === dep.id && (
                    <div style={moveBar}>
                      переместить в:
                      <select style={{ marginLeft: 8 }} defaultValue="" onChange={(e) => doMoveDepartment(dep.id, e.target.value)}>
                        <option value="" disabled>выбрать отделение…</option>
                        {tree.divisions.map((td) => (
                          <option key={td.id} value={td.id}>{td.name}</option>
                        ))}
                      </select>
                      <button type="button" style={iconBtn} onClick={() => setMove(null)}>отмена</button>
                    </div>
                  )}

                  {dep.agents.map((a) => {
                    const sm = STATUS_META[a.status] ?? { label: a.status, color: "#666" };
                    const isSel = selected?.id === a.id;
                    return (
                      <div key={a.id} style={{ marginLeft: 22, marginTop: 4 }}>
                        <div
                          style={{ ...nodeBox, cursor: "pointer", background: isSel ? "#fff7e6" : "#fafafa", borderColor: isSel ? "#e0b050" : "#e2e2e2" }}
                          onClick={() => setSelected(a)}
                        >
                          <span style={{ fontSize: 13 }}>{a.role_label === "head" ? "⭐" : "🤖"}</span>
                          <span>{a.name}</span>
                          {a.role_label === "head" && <span style={badge("#6b4fb0")}>глава</span>}
                          <span style={badge(sm.color)}>{sm.label}</span>
                          <span style={rowActions}>
                            <button type="button" style={iconBtn} title="Переименовать" onClick={(e) => { e.stopPropagation(); renameAgent(a); }}>✏</button>
                            <button type="button" style={iconBtn} title="Переместить в другой отдел" onClick={(e) => { e.stopPropagation(); setMove({ kind: "agent", id: a.id }); }}>↗</button>
                            <button type="button" style={iconBtn} title="Удалить" onClick={(e) => { e.stopPropagation(); deleteAgent(a); }}>🗑</button>
                          </span>
                        </div>
                        {move?.kind === "agent" && move.id === a.id && (
                          <div style={moveBar}>
                            переместить в:
                            <select style={{ marginLeft: 8 }} defaultValue="" onChange={(e) => doMoveAgent(a.id, e.target.value)}>
                              <option value="" disabled>выбрать отдел…</option>
                              {allDepartments.map((od) => (
                                <option key={od.id} value={od.id}>{od.label}</option>
                              ))}
                            </select>
                            <button type="button" style={iconBtn} onClick={() => setMove(null)}>отмена</button>
                          </div>
                        )}
                      </div>
                    );
                  })}
                </div>
              ))}
            </div>
          ))
        )}
      </div>

      {/* ПРАВАЯ ПАНЕЛЬ — карточка агента (заглушка, полноценно в Заходе 2) */}
      <div style={{ flex: "1 1 40%", overflowY: "auto", padding: "24px 28px", background: "#fff" }}>
        <h2 style={{ marginTop: 0, fontSize: 18 }}>Карточка агента</h2>
        {selected == null ? (
          <p style={{ color: "#888", fontSize: 13 }}>Выбери агента в дереве слева.</p>
        ) : (
          <div style={{ fontSize: 13, lineHeight: 1.7 }}>
            <div><strong>Имя:</strong> {selected.name}</div>
            <div><strong>Slug:</strong> <code>{selected.slug}</code></div>
            <div><strong>Роль:</strong> {selected.role_label === "head" ? "глава" : "обычный"}</div>
            <div><strong>Статус:</strong> {(STATUS_META[selected.status] ?? { label: selected.status }).label}</div>
            <div><strong>Папка:</strong> {selected.folder_path ?? "— (создаётся на следующем этапе)"}</div>
            <div style={{ marginTop: 16, padding: 12, background: "#f5f5f5", borderRadius: 4, color: "#666" }}>
              ✏ Редактор карточки (роль/мозг/MCP/CLAUDE.md/память/ЦКП/чек-лист) — <strong>Заход 2</strong>.
              Сейчас это скелет: структура и операции дерева.
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

// ----- inline styles -----
const primaryBtn: React.CSSProperties = {
  padding: "6px 14px", background: "#1a1a2e", color: "#fff", border: "none",
  borderRadius: 4, cursor: "pointer", fontSize: 13,
};
const iconBtn: React.CSSProperties = {
  padding: "2px 8px", background: "#fff", border: "1px solid #ccc",
  borderRadius: 3, cursor: "pointer", fontSize: 11, marginLeft: 4,
};
const nodeBox: React.CSSProperties = {
  display: "flex", alignItems: "center", gap: 8, padding: "8px 12px",
  border: "1px solid #ddd", borderRadius: 6, fontSize: 14,
};
const rowActions: React.CSSProperties = { marginLeft: "auto", display: "flex", alignItems: "center" };
const moveBar: React.CSSProperties = {
  marginLeft: 22, marginTop: 4, padding: "6px 10px", background: "#eef6ff",
  border: "1px solid #bcd8f5", borderRadius: 4, fontSize: 12,
  display: "flex", alignItems: "center", gap: 4,
};
function badge(color: string): React.CSSProperties {
  return { fontSize: 10, fontWeight: 600, color, border: `1px solid ${color}`, borderRadius: 8, padding: "1px 6px", marginLeft: 6 };
}
