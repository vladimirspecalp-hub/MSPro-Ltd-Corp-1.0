import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import FileActions from "../common/FileActions";

interface ResultArtifact {
  id: string;
  rel_path: string;
  created_by: string;
}

interface Props {
  /** id РОДИТЕЛЬСКОЙ ceo→dispatcher задачи. Бэкенд (list_result_artifacts)
   *  резолвит её прямых детей (office-manager) и отдаёт их не-отклонённые
   *  артефакты — сам чат знает только родителя. */
  parentTaskId: string;
}

/**
 * BL-P1-018 Заход 2 — файлы-результат прямо под системным (⚡) сообщением
 * в чате Гендира. Появляются живьём: при завершении пост-агента
 * (`post-executor-finished`) список перезапрашивается. Пусто → ничего не
 * рисуем (тихо), чтобы не зашумлять чат у задач без артефактов.
 */
export default function ChatTaskArtifacts({ parentTaskId }: Props) {
  const [list, setList] = useState<ResultArtifact[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    async function refresh() {
      try {
        const data = await invoke<ResultArtifact[]>("list_result_artifacts", {
          taskId: parentTaskId,
        });
        if (alive) {
          setList(data);
          setError(null);
        }
      } catch (e) {
        if (alive) setError(String(e));
      }
    }
    refresh();
    // Живое появление: пост-агент завершился → перезапрашиваем артефакты.
    const un = listen("post-executor-finished", () => refresh());
    return () => {
      alive = false;
      un.then((f) => f());
    };
  }, [parentTaskId]);

  if (error) {
    return (
      <div style={{ fontSize: 11, color: "#c00", marginTop: 6 }}>
        Не удалось загрузить файлы: {error}
      </div>
    );
  }
  if (list.length === 0) {
    return null; // тихо — артефактов ещё (или совсем) нет
  }

  return (
    <div
      style={{
        marginTop: 8,
        paddingTop: 8,
        borderTop: "1px solid #eee",
        display: "flex",
        flexDirection: "column",
        gap: 6,
      }}
    >
      <div style={{ fontSize: 11, color: "#666", fontWeight: 600 }}>
        📎 Файлы-результат:
      </div>
      {list.map((a) => (
        <div
          key={a.id}
          style={{
            display: "flex",
            alignItems: "center",
            gap: 8,
            flexWrap: "wrap",
          }}
        >
          <code style={{ fontSize: 11 }}>{a.rel_path}</code>
          <FileActions artifactId={a.id} compact onError={setError} />
        </div>
      ))}
    </div>
  );
}
