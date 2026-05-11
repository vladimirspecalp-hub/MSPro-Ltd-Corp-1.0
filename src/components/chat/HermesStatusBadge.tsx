import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type HermesStatus =
  | { kind: "available"; distro: string; version: string; skill_path: string | null }
  | { kind: "skill_missing"; distro: string; version: string; configured_skill: string }
  | { kind: "hermes_not_installed"; distro: string }
  | { kind: "distro_not_found"; configured_distro: string; available: string[] }
  | { kind: "wsl_not_available"; error: string };

interface Props {
  /** Called whenever status changes so the parent can enable/disable input. */
  onStatusChange?: (s: HermesStatus | null) => void;
}

const INSTALL_CMD =
  "curl -fsSL https://raw.githubusercontent.com/NousResearch/hermes-agent/main/scripts/install.sh | bash";

const SKILL_BOOTSTRAP = `mkdir -p ~/.hermes/skills/ceo
cat > ~/.hermes/skills/ceo/SKILL.md <<'EOF'
---
name: ceo
description: CEO agent for MSPro-Ltd Corp. Reads {system, user} from stdin JSON.
---
You are the CEO of MSPro-Ltd Corp. The host application passes a JSON
envelope on stdin: {"system": "<context>", "user": "<question>"}.
Read both, prepend the system context to your reasoning, then answer
the user message in Russian.
EOF`;

const wrapStyle: React.CSSProperties = {
  padding: "8px 14px",
  borderRadius: 6,
  border: "1px solid",
  fontSize: 12,
  display: "flex",
  alignItems: "center",
  gap: 10,
  flexWrap: "wrap",
};

const badgeColors: Record<string, { bg: string; border: string; fg: string; emoji: string }> = {
  available: { bg: "#e8f5e9", border: "#4caf50", fg: "#1b5e20", emoji: "🟢" },
  skill_missing: { bg: "#fff8e1", border: "#ffa000", fg: "#e65100", emoji: "🟡" },
  hermes_not_installed: { bg: "#fee", border: "#c00", fg: "#7a0000", emoji: "🔴" },
  distro_not_found: { bg: "#fee", border: "#c00", fg: "#7a0000", emoji: "🔴" },
  wsl_not_available: { bg: "#fee", border: "#c00", fg: "#7a0000", emoji: "🔴" },
  loading: { bg: "#fafafa", border: "#ccc", fg: "#666", emoji: "⏳" },
};

function describe(status: HermesStatus): string {
  switch (status.kind) {
    case "available":
      return `Hermes готов · ${status.distro} · ${status.version}${
        status.skill_path ? ` · skill /ceo OK` : ""
      }`;
    case "skill_missing":
      return `Hermes есть (${status.distro} · ${status.version}), но skill ${status.configured_skill} не найден`;
    case "hermes_not_installed":
      return `Hermes не установлен в WSL distro «${status.distro}»`;
    case "distro_not_found":
      return `WSL distro «${status.configured_distro}» не найдена. Доступны: ${
        status.available.join(", ") || "(пусто)"
      }`;
    case "wsl_not_available":
      return `WSL недоступен: ${status.error || "unknown"}`;
  }
}

async function copy(text: string) {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    /* ignore */
  }
}

export default function HermesStatusBadge({ onStatusChange }: Props) {
  const [status, setStatus] = useState<HermesStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [showGuide, setShowGuide] = useState(false);

  async function refresh() {
    setLoading(true);
    setError(null);
    try {
      const s = await invoke<HermesStatus>("detect_hermes_status");
      setStatus(s);
      onStatusChange?.(s);
    } catch (e) {
      setError(String(e));
      onStatusChange?.(null);
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    refresh();
  }, []);

  if (loading) {
    const c = badgeColors.loading;
    return (
      <div style={{ ...wrapStyle, background: c.bg, borderColor: c.border, color: c.fg }}>
        <span>{c.emoji}</span> Проверяю Hermes…
      </div>
    );
  }

  if (error) {
    return (
      <div style={{ ...wrapStyle, background: "#fee", borderColor: "#c00", color: "#7a0000" }}>
        ⚠️ Ошибка детекции: {error}
      </div>
    );
  }

  if (!status) return null;

  const c = badgeColors[status.kind] ?? badgeColors.wsl_not_available;
  const isProblem = status.kind !== "available";
  const expandable = status.kind !== "available";

  return (
    <div>
      <div style={{ ...wrapStyle, background: c.bg, borderColor: c.border, color: c.fg }}>
        <span>{c.emoji}</span>
        <span style={{ flex: 1 }}>{describe(status)}</span>
        <button
          type="button"
          onClick={refresh}
          style={{
            padding: "2px 10px",
            background: "transparent",
            border: `1px solid ${c.border}`,
            color: c.fg,
            borderRadius: 4,
            cursor: "pointer",
            fontSize: 11,
          }}
        >
          ↻ Перепроверить
        </button>
        {expandable && (
          <button
            type="button"
            onClick={() => setShowGuide((v) => !v)}
            style={{
              padding: "2px 10px",
              background: c.fg,
              border: "none",
              color: "#fff",
              borderRadius: 4,
              cursor: "pointer",
              fontSize: 11,
            }}
          >
            {showGuide ? "Скрыть" : "Как починить"}
          </button>
        )}
      </div>

      {showGuide && isProblem && (
        <div
          style={{
            marginTop: 8,
            padding: 14,
            border: "1px solid #ddd",
            borderRadius: 6,
            background: "#fafafa",
            fontSize: 12,
            color: "#333",
          }}
        >
          {status.kind === "wsl_not_available" && (
            <>
              <p>
                <strong>WSL не запущен или не установлен.</strong>
              </p>
              <p>
                В PowerShell от админа:{" "}
                <code style={codeStyle}>wsl --install -d Ubuntu</code>
              </p>
              <p>После установки перезагрузи компьютер и зайди в WSL чтобы создать пользователя.</p>
            </>
          )}

          {status.kind === "distro_not_found" && (
            <>
              <p>
                <strong>Доступные distro:</strong> {status.available.join(", ") || "(пусто)"}
              </p>
              <p>
                Открой <em>Настройки → Hermes</em> и выбери одну из них в поле «Distro» (мы добавим
                выпадашку в Шаге 5; пока правится через JSON в{" "}
                <code style={codeStyle}>%APPDATA%\Roaming\ru.msproltd.corp\settings.json</code>).
              </p>
            </>
          )}

          {status.kind === "hermes_not_installed" && (
            <>
              <p>
                <strong>В WSL distro «{status.distro}» нет Hermes.</strong> Выполни одной строкой:
              </p>
              <pre style={preStyle}>{INSTALL_CMD}</pre>
              <button type="button" onClick={() => copy(INSTALL_CMD)} style={copyBtnStyle}>
                Скопировать команду
              </button>
              <p style={{ marginTop: 10 }}>
                После установки пройди мастер <code style={codeStyle}>hermes setup</code>, затем
                нажми «↻ Перепроверить».
              </p>
            </>
          )}

          {status.kind === "skill_missing" && (
            <>
              <p>
                <strong>Hermes установлен, но skill {status.configured_skill} не найден.</strong>{" "}
                Создай skill в WSL:
              </p>
              <pre style={preStyle}>{SKILL_BOOTSTRAP}</pre>
              <button type="button" onClick={() => copy(SKILL_BOOTSTRAP)} style={copyBtnStyle}>
                Скопировать
              </button>
            </>
          )}
        </div>
      )}
    </div>
  );
}

const codeStyle: React.CSSProperties = {
  background: "#eee",
  padding: "1px 6px",
  borderRadius: 3,
  fontFamily: "ui-monospace, monospace",
  fontSize: 11,
};

const preStyle: React.CSSProperties = {
  background: "#1a1a1a",
  color: "#e0e0e0",
  padding: 12,
  borderRadius: 4,
  overflowX: "auto",
  fontSize: 11,
  fontFamily: "ui-monospace, monospace",
  margin: "8px 0",
  whiteSpace: "pre-wrap",
};

const copyBtnStyle: React.CSSProperties = {
  padding: "6px 12px",
  background: "#1a1a1a",
  color: "#fff",
  border: "none",
  borderRadius: 4,
  cursor: "pointer",
  fontSize: 11,
  fontWeight: 600,
};
