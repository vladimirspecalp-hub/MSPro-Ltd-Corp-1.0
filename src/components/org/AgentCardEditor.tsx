import { useEffect, useState, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface AgentCard {
  id: string;
  name: string;
  slug: string;
  role_label: string;
  status: string;
  role_prompt_md: string | null;
  brain_mode: string;
  brain_model: string | null;
  brain_endpoint: string | null;
  mcp_servers_json: string;
  ckp_text: string | null;
  checklist_json: string;
  memory_md: string | null;
}

export interface AgentLink {
  id: string;
  from_agent_id: string;
  to_agent_id: string;
  link_type: string;
  description: string | null;
  sort_order: number;
  created_at: string;
}

interface VaultMeta {
  id: string;
  key_name: string;
  description: string | null;
  access_level: number;
  credential_target: string;
  created_at: string;
  updated_at: string;
}

interface Props {
  agentId: string;
  allAgents: Array<{ id: string; name: string }>;
}

const BRAIN_MODES: Array<{ value: string; label: string }> = [
  { value: "disabled", label: "Выключен" },
  { value: "claude_cli", label: "Claude CLI" },
  { value: "qwen_http", label: "Qwen HTTP" },
  { value: "external_gateway", label: "External Gateway" },
];

function shortId(agentId: string): string {
  return agentId.substring(4, 12);
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export default function AgentCardEditor({ agentId, allAgents }: Props) {
  const [card, setCard] = useState<AgentCard | null>(null);
  const [links, setLinks] = useState<AgentLink[]>([]);
  const [secrets, setSecrets] = useState<VaultMeta[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [dirty, setDirty] = useState(false);

  // Form fields
  const [rolePrompt, setRolePrompt] = useState("");
  const [brainMode, setBrainMode] = useState("disabled");
  const [brainModel, setBrainModel] = useState("");
  const [brainEndpoint, setBrainEndpoint] = useState("");
  const [mcpServers, setMcpServers] = useState("[]");
  const [ckpText, setCkpText] = useState("");
  const [checklistJson, setChecklistJson] = useState("[]");
  const [memoryMd, setMemoryMd] = useState("");

  const prefix = `agent-${shortId(agentId)}-`;

  const loadCard = useCallback(async () => {
    try {
      const c = await invoke<AgentCard>("agent_card_get", { agentId });
      setCard(c);
      setRolePrompt(c.role_prompt_md ?? "");
      setBrainMode(c.brain_mode);
      setBrainModel(c.brain_model ?? "");
      setBrainEndpoint(c.brain_endpoint ?? "");
      setMcpServers(c.mcp_servers_json);
      setCkpText(c.ckp_text ?? "");
      setChecklistJson(c.checklist_json);
      setMemoryMd(c.memory_md ?? "");
      setDirty(false);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, [agentId]);

  const loadLinks = useCallback(async () => {
    try {
      const l = await invoke<AgentLink[]>("agent_links_get", { agentId });
      setLinks(l);
    } catch (e) {
      setError(String(e));
    }
  }, [agentId]);

  const loadSecrets = useCallback(async () => {
    try {
      const all = await invoke<VaultMeta[]>("vault_list_secrets");
      setSecrets(all.filter((s) => s.key_name.startsWith(prefix)));
    } catch (e) {
      setError(String(e));
    }
  }, [prefix]);

  useEffect(() => {
    loadCard();
    loadLinks();
    loadSecrets();
  }, [loadCard, loadLinks, loadSecrets]);

  async function save() {
    setSaving(true);
    try {
      const updated = await invoke<AgentCard>("agent_card_save", {
        agentId,
        input: {
          role_prompt_md: rolePrompt || null,
          brain_mode: brainMode,
          brain_model: brainModel || null,
          brain_endpoint: brainEndpoint || null,
          mcp_servers_json: mcpServers,
          ckp_text: ckpText || null,
          checklist_json: checklistJson,
          memory_md: memoryMd || null,
        },
      });
      setCard(updated);
      setDirty(false);
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  }

  // --- Links ---
  const outNext = links.filter((l) => l.from_agent_id === agentId && l.link_type === "next");
  const outVerifier = links.filter((l) => l.from_agent_id === agentId && l.link_type === "verifier");
  const incomingNext = links.filter((l) => l.to_agent_id === agentId && l.link_type === "next");

  async function addLink(toId: string, linkType: string) {
    if (!toId) return;
    try {
      await invoke("agent_link_set", {
        fromAgentId: agentId,
        toAgentId: toId,
        linkType,
        description: null,
      });
      setError(null);
      await loadLinks();
    } catch (e) {
      setError(String(e));
    }
  }

  async function removeLink(linkId: string) {
    try {
      await invoke("agent_link_remove", { linkId });
      setError(null);
      await loadLinks();
    } catch (e) {
      setError(String(e));
    }
  }

  // --- Secrets ---
  async function addSecret() {
    const name = window.prompt(`Имя секрета (без префикса, будет: ${prefix}...):`);
    if (!name?.trim()) return;
    const value = window.prompt("Значение секрета:");
    if (value == null) return;
    try {
      await invoke("vault_add_secret", {
        input: {
          key_name: `${prefix}${name.trim()}`,
          value,
          description: `Agent ${card?.name ?? agentId}`,
          access_level: 2,
        },
      });
      setError(null);
      await loadSecrets();
    } catch (e) {
      setError(String(e));
    }
  }

  async function removeSecret(keyName: string) {
    if (!window.confirm(`Удалить секрет ${keyName}?`)) return;
    try {
      await invoke("vault_remove_secret", { keyName });
      await loadSecrets();
    } catch (e) {
      setError(String(e));
    }
  }

  const otherAgents = allAgents.filter((a) => a.id !== agentId);
  const agentName = (id: string) => allAgents.find((a) => a.id === id)?.name ?? id;

  if (!card) {
    return <div style={{ color: "#888", fontSize: 13 }}>loading...</div>;
  }

  return (
    <div style={{ fontSize: 13, lineHeight: 1.7 }}>
      {/* Header */}
      <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 12 }}>
        <h2 style={{ margin: 0, fontSize: 18 }}>{card.name}</h2>
        <code style={{ color: "#888", fontSize: 11 }}>{card.slug}</code>
        <span style={badgeStyle(card.role_label === "head" ? "#6b4fb0" : "#888")}>
          {card.role_label === "head" ? "глава" : "member"}
        </span>
      </div>

      {error && (
        <div style={errorBox}>{error}</div>
      )}

      {/* === СВЯЗИ В ПОТОКЕ (mini-schema) === */}
      <Section title="Связи в потоке">
        <div style={{ display: "flex", alignItems: "center", justifyContent: "center", gap: 8, padding: "12px 0", flexWrap: "wrap" }}>
          {/* input_from */}
          <div style={flowBox("#e8f4fd")}>
            <div style={{ fontSize: 10, color: "#666", marginBottom: 2 }}>input</div>
            {incomingNext.length > 0
              ? incomingNext.map((l) => (
                  <div key={l.id} style={{ fontSize: 11 }}>{agentName(l.from_agent_id)}</div>
                ))
              : <div style={{ fontSize: 11, color: "#aaa" }}>--</div>
            }
          </div>
          <span style={{ fontSize: 16, color: "#999" }}>&rarr;</span>

          {/* current agent */}
          <div style={{ ...flowBox("#fff7e6"), borderColor: "#e0b050", fontWeight: 600, position: "relative" }}>
            {card.name}
            {/* verifier above */}
            {outVerifier.length > 0 && (
              <div style={{ position: "absolute", top: -28, left: "50%", transform: "translateX(-50%)", whiteSpace: "nowrap" }}>
                {outVerifier.map((v) => (
                  <span key={v.id} style={{ ...badgeStyle("#2d8a4e"), fontSize: 9 }}>
                    verifier: {agentName(v.to_agent_id)}
                  </span>
                ))}
              </div>
            )}
          </div>
          <span style={{ fontSize: 16, color: "#999" }}>&rarr;</span>

          {/* next */}
          <div style={flowBox("#e8fde8")}>
            <div style={{ fontSize: 10, color: "#666", marginBottom: 2 }}>next</div>
            {outNext.length > 0
              ? outNext.map((l) => (
                  <div key={l.id} style={{ fontSize: 11 }}>{agentName(l.to_agent_id)}</div>
                ))
              : <div style={{ fontSize: 11, color: "#aaa" }}>--</div>
            }
          </div>
        </div>

        {/* link editors */}
        <div style={{ display: "flex", gap: 16, flexWrap: "wrap" }}>
          <div>
            <label style={labelStyle}>Next &rarr;</label>
            {outNext.map((l) => (
              <div key={l.id} style={{ display: "flex", gap: 4, alignItems: "center", marginBottom: 2 }}>
                <span>{agentName(l.to_agent_id)}</span>
                <button type="button" style={smallBtn} onClick={() => removeLink(l.id)}>x</button>
              </div>
            ))}
            <select
              style={selectStyle}
              value=""
              onChange={(e) => addLink(e.target.value, "next")}
            >
              <option value="" disabled>+ добавить next...</option>
              {otherAgents.map((a) => (
                <option key={a.id} value={a.id}>{a.name}</option>
              ))}
            </select>
          </div>
          <div>
            <label style={labelStyle}>Verifier</label>
            {outVerifier.map((l) => (
              <div key={l.id} style={{ display: "flex", gap: 4, alignItems: "center", marginBottom: 2 }}>
                <span>{agentName(l.to_agent_id)}</span>
                <button type="button" style={smallBtn} onClick={() => removeLink(l.id)}>x</button>
              </div>
            ))}
            <select
              style={selectStyle}
              value=""
              onChange={(e) => addLink(e.target.value, "verifier")}
            >
              <option value="" disabled>+ добавить verifier...</option>
              {otherAgents.map((a) => (
                <option key={a.id} value={a.id}>{a.name}</option>
              ))}
            </select>
          </div>
          <div>
            <label style={labelStyle}>Input from (read-only)</label>
            {incomingNext.length > 0
              ? incomingNext.map((l) => (
                  <div key={l.id} style={{ fontSize: 12, color: "#555" }}>{agentName(l.from_agent_id)}</div>
                ))
              : <div style={{ fontSize: 12, color: "#aaa" }}>нет входящих</div>
            }
          </div>
        </div>
      </Section>

      {/* === Роль + ЦКП === */}
      <Section title="Роль и ЦКП">
        <label style={labelStyle}>Роль (system prompt)</label>
        <textarea
          style={textareaStyle}
          rows={5}
          value={rolePrompt}
          onChange={(e) => { setRolePrompt(e.target.value); setDirty(true); }}
          placeholder="Описание роли агента (Markdown)..."
        />
        <label style={labelStyle}>ЦКП (ценный конечный продукт)</label>
        <input
          style={inputStyle}
          value={ckpText}
          onChange={(e) => { setCkpText(e.target.value); setDirty(true); }}
          placeholder="Что агент производит..."
        />
      </Section>

      {/* === Мозг === */}
      <Section title="Мозг">
        <label style={labelStyle}>Режим мозга</label>
        <select
          style={selectStyle}
          value={brainMode}
          onChange={(e) => { setBrainMode(e.target.value); setDirty(true); }}
        >
          {BRAIN_MODES.map((m) => (
            <option key={m.value} value={m.value}>{m.label}</option>
          ))}
        </select>
        {brainMode !== "disabled" && (
          <>
            <label style={labelStyle}>Модель</label>
            <input
              style={inputStyle}
              value={brainModel}
              onChange={(e) => { setBrainModel(e.target.value); setDirty(true); }}
              placeholder="opus / qwen3:14b / ..."
            />
          </>
        )}
        {brainMode === "qwen_http" && (
          <>
            <label style={labelStyle}>Endpoint</label>
            <input
              style={inputStyle}
              value={brainEndpoint}
              onChange={(e) => { setBrainEndpoint(e.target.value); setDirty(true); }}
              placeholder="http://localhost:11434/v1"
            />
          </>
        )}
      </Section>

      {/* === MCP серверы === */}
      <Section title="MCP серверы">
        <textarea
          style={textareaStyle}
          rows={3}
          value={mcpServers}
          onChange={(e) => { setMcpServers(e.target.value); setDirty(true); }}
          placeholder='["context7","sequential-thinking"]'
        />
      </Section>

      {/* === Память + Чек-лист === */}
      <Section title="Память и Чек-лист">
        <label style={labelStyle}>Память (Markdown)</label>
        <textarea
          style={textareaStyle}
          rows={4}
          value={memoryMd}
          onChange={(e) => { setMemoryMd(e.target.value); setDirty(true); }}
          placeholder="Заметки, инструкции, контекст..."
        />
        <label style={labelStyle}>Чек-лист (JSON)</label>
        <textarea
          style={textareaStyle}
          rows={3}
          value={checklistJson}
          onChange={(e) => { setChecklistJson(e.target.value); setDirty(true); }}
          placeholder='["Проверить отчёт","Согласовать с юристом"]'
        />
      </Section>

      {/* === Секреты === */}
      <Section title="Секреты (Vault)">
        <div style={{ fontSize: 11, color: "#888", marginBottom: 6 }}>
          Ключи с префиксом <code>{prefix}</code>
        </div>
        {secrets.length === 0 ? (
          <div style={{ fontSize: 12, color: "#aaa" }}>Нет секретов</div>
        ) : (
          secrets.map((s) => (
            <div key={s.id} style={{ display: "flex", gap: 6, alignItems: "center", marginBottom: 3 }}>
              <code style={{ fontSize: 11 }}>{s.key_name}</code>
              <span style={{ fontSize: 10, color: "#888" }}>{s.description}</span>
              <button type="button" style={smallBtn} onClick={() => removeSecret(s.key_name)}>x</button>
            </div>
          ))
        )}
        <button type="button" style={addBtn} onClick={addSecret}>+ секрет</button>
      </Section>

      {/* === Save === */}
      <div style={{ position: "sticky", bottom: 0, background: "#fff", padding: "10px 0", borderTop: "1px solid #eee", display: "flex", gap: 8 }}>
        <button
          type="button"
          style={{ ...saveBtn, opacity: dirty ? 1 : 0.5 }}
          disabled={!dirty || saving}
          onClick={save}
        >
          {saving ? "..." : "Сохранить"}
        </button>
        {dirty && <span style={{ fontSize: 11, color: "#c07000", alignSelf: "center" }}>есть несохранённые изменения</span>}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Section helper
// ---------------------------------------------------------------------------

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <details open style={{ marginBottom: 10, borderBottom: "1px solid #f0f0f0", paddingBottom: 8 }}>
      <summary style={{ cursor: "pointer", fontWeight: 600, fontSize: 13, color: "#333", marginBottom: 6 }}>
        {title}
      </summary>
      {children}
    </details>
  );
}

// ---------------------------------------------------------------------------
// Styles
// ---------------------------------------------------------------------------

const labelStyle: React.CSSProperties = {
  display: "block", fontSize: 11, color: "#666", marginTop: 6, marginBottom: 2,
};
const inputStyle: React.CSSProperties = {
  width: "100%", padding: "5px 8px", border: "1px solid #ddd", borderRadius: 4,
  fontSize: 13, boxSizing: "border-box",
};
const textareaStyle: React.CSSProperties = {
  ...inputStyle, fontFamily: "monospace", resize: "vertical",
};
const selectStyle: React.CSSProperties = {
  padding: "4px 8px", border: "1px solid #ddd", borderRadius: 4, fontSize: 12,
};
const smallBtn: React.CSSProperties = {
  padding: "1px 6px", border: "1px solid #ccc", borderRadius: 3, background: "#fff",
  cursor: "pointer", fontSize: 10, color: "#a00",
};
const addBtn: React.CSSProperties = {
  marginTop: 6, padding: "4px 12px", border: "1px solid #ccc", borderRadius: 4,
  background: "#fafafa", cursor: "pointer", fontSize: 12,
};
const saveBtn: React.CSSProperties = {
  padding: "6px 20px", background: "#1a1a2e", color: "#fff", border: "none",
  borderRadius: 4, cursor: "pointer", fontSize: 13,
};
const errorBox: React.CSSProperties = {
  padding: 8, background: "#fff2f0", color: "#a51b1b", borderRadius: 4, fontSize: 12,
  marginBottom: 10,
};
function badgeStyle(color: string): React.CSSProperties {
  return {
    fontSize: 10, fontWeight: 600, color, border: `1px solid ${color}`,
    borderRadius: 8, padding: "1px 6px", marginLeft: 4,
  };
}
function flowBox(bg: string): React.CSSProperties {
  return {
    padding: "6px 12px", border: "1px solid #ddd", borderRadius: 6, background: bg,
    textAlign: "center", minWidth: 60, position: "relative",
  };
}
