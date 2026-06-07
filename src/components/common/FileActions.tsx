import { invoke } from "@tauri-apps/api/core";

interface Props {
  /** id артефакта (task_artifacts.id). Бэкенд сам резолвит абсолютный путь и
   *  проверяет, что он внутри Outbox (safe_artifact_path) — фронт произвольный
   *  путь подсунуть не может. */
  artifactId: string;
  /** колбэк ошибки; если не задан — alert. */
  onError?: (msg: string) => void;
  /** компактный вид (для чата) — меньше отступы/шрифт. */
  compact?: boolean;
}

/**
 * BL-P1-018 — единый переиспользуемый компонент «действия с файлом-результатом».
 * Два действия: «Открыть файл» (в программе по умолчанию) и «Открыть папку»
 * (Проводник Windows). Один компонент на все места: сейчас — Диспетчер
 * (ArtifactsPanel); далее без изменений встанет в чат Гендира и чат агента
 * (отдельные заходы) — отсюда самодостаточность (только artifactId) + `compact`.
 */
export default function FileActions({ artifactId, onError, compact = false }: Props) {
  async function run(
    e: React.MouseEvent,
    cmd: "open_artifact_in_default_app" | "open_artifact_folder",
  ) {
    e.stopPropagation();
    try {
      await invoke(cmd, { artifactId });
    } catch (err) {
      const msg = String(err);
      if (onError) onError(msg);
      else alert(msg);
    }
  }

  const btn: React.CSSProperties = {
    padding: compact ? "2px 8px" : "4px 10px",
    background: "#fff",
    border: "1px solid #ccc",
    borderRadius: 3,
    cursor: "pointer",
    fontSize: compact ? 11 : 12,
  };

  return (
    <div style={{ display: "flex", gap: 6 }}>
      <button
        type="button"
        onClick={(e) => run(e, "open_artifact_in_default_app")}
        style={btn}
        title="Открыть файл в программе по умолчанию"
      >
        📄 Открыть файл
      </button>
      <button
        type="button"
        onClick={(e) => run(e, "open_artifact_folder")}
        style={btn}
        title="Открыть папку в Проводнике"
      >
        📂 Открыть папку
      </button>
    </div>
  );
}
