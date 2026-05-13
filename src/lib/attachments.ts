// Шаг 8 «Глаза Гендира» — клиентская обработка прикреплённых файлов и папок.
// Текстовые форматы читаются через File.text() (UTF-8). Бинарные отображаются
// в UI с пометкой «не поддерживается», но в send_chat_message не идут.

/** Расширения, которые считаются «текстовыми» и попадают в prompt. */
export const TEXT_EXTS = new Set<string>([
  "md", "txt", "rst",
  "json", "yaml", "yml", "toml", "ini", "cfg", "conf",
  "csv", "tsv",
  "py", "ts", "tsx", "js", "jsx", "mjs", "cjs",
  "rs", "go", "java", "kt", "c", "cpp", "cc", "h", "hpp",
  "sh", "bash", "ps1", "bat", "cmd",
  "sql", "html", "htm", "xml", "svg",
  "log", "env",
  "vue", "svelte", "lua", "rb", "php",
]);

/** Сегменты пути, которые игнорируем при обходе папки. */
const IGNORE_SEGMENTS = new Set<string>([
  "node_modules", ".git", "target", "dist", "build", ".next",
  "__pycache__", ".venv", "venv", ".pytest_cache",
  ".idea", ".vscode", ".DS_Store",
]);

export const PER_FILE_MAX = 200 * 1024;    // 200 KB
export const TOTAL_MAX = 1024 * 1024;      // 1 MB total
export const MAX_FILES = 50;

export type AttachmentKind = "file" | "folder-file" | "unsupported";

export interface AttachmentItem {
  /** Уникальный id внутри сессии (для удаления). */
  id: string;
  /** Базовое имя файла. */
  filename: string;
  /** Относительный путь внутри выбранной папки (для folder-file). */
  relativePath?: string;
  /** Размер в байтах (фактически прочитанный — после возможной обрезки). */
  sizeBytes: number;
  /** Was content truncated to PER_FILE_MAX? */
  truncated: boolean;
  /** Тип — для UI rendering и фильтра при отправке. */
  kind: AttachmentKind;
  /** Содержимое (для kind != "unsupported"). */
  textContent: string;
  /** Причина если unsupported (для UI подсказки). */
  unsupportedReason?: string;
}

/** Группировка в UI: одиночные файлы + папки. */
export interface FolderBundle {
  rootName: string;
  itemIds: string[]; // ids в `AttachmentItem[]` которые входят в эту папку
}

function getExtension(name: string): string {
  const i = name.lastIndexOf(".");
  if (i < 0 || i === name.length - 1) return "";
  return name.slice(i + 1).toLowerCase();
}

function isPathIgnored(relPath: string): boolean {
  return relPath.split(/[\\/]/).some((seg) => IGNORE_SEGMENTS.has(seg));
}

function newId(): string {
  return `att-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
}

/** Полу-безопасное декодирование UTF-8 с подстановкой `�` на битых байтах. */
async function readTextSafe(file: File): Promise<string> {
  const buf = await file.arrayBuffer();
  const decoder = new TextDecoder("utf-8", { fatal: false });
  return decoder.decode(buf);
}

/**
 * Читает одиночный File. Возвращает AttachmentItem либо undefined если
 * файл должен быть полностью отброшен (например пустое имя).
 */
export async function readSingleFile(file: File): Promise<AttachmentItem> {
  const filename = file.name;
  const ext = getExtension(filename);
  const id = newId();

  if (!TEXT_EXTS.has(ext)) {
    return {
      id,
      filename,
      sizeBytes: file.size,
      truncated: false,
      kind: "unsupported",
      textContent: "",
      unsupportedReason: `Расширение .${ext || "?"} не поддерживается. Сконвертируй в .md / .txt через manager-skill.`,
    };
  }

  const raw = await readTextSafe(file);
  const truncated = raw.length > PER_FILE_MAX;
  const text = truncated
    ? raw.slice(0, PER_FILE_MAX) + "\n… [обрезано, оригинальный файл больше 200 KB]"
    : raw;

  return {
    id,
    filename,
    sizeBytes: text.length,
    truncated,
    kind: "file",
    textContent: text,
  };
}

/**
 * Обрабатывает FileList от `<input type="file" webkitdirectory>`.
 * Возвращает items + bundle описание (для одного root folder).
 */
export async function readFolderFiles(
  files: FileList,
): Promise<{ items: AttachmentItem[]; folderBundle: FolderBundle | null; skipped: number }> {
  if (files.length === 0) return { items: [], folderBundle: null, skipped: 0 };

  // 1) Отфильтровать по whitelist + ignore segments + отсортировать
  const candidates: File[] = [];
  let skipped = 0;
  for (let i = 0; i < files.length; i++) {
    const f = files[i];
    const relPath = (f as File & { webkitRelativePath?: string }).webkitRelativePath || f.name;
    if (isPathIgnored(relPath)) {
      skipped++;
      continue;
    }
    const ext = getExtension(f.name);
    if (!TEXT_EXTS.has(ext)) {
      skipped++;
      continue;
    }
    candidates.push(f);
  }

  // 2) Лексикографически по relPath чтобы Гендиру было приятно читать
  candidates.sort((a, b) => {
    const ap = (a as File & { webkitRelativePath?: string }).webkitRelativePath || a.name;
    const bp = (b as File & { webkitRelativePath?: string }).webkitRelativePath || b.name;
    return ap.localeCompare(bp);
  });

  // 3) Cap MAX_FILES
  const overflowSkipped = Math.max(0, candidates.length - MAX_FILES);
  const taken = candidates.slice(0, MAX_FILES);

  // 4) Прочитать каждый, копить total size
  const items: AttachmentItem[] = [];
  let total = 0;
  for (const f of taken) {
    const relPath = (f as File & { webkitRelativePath?: string }).webkitRelativePath || f.name;
    const raw = await readTextSafe(f);
    const remainingBudget = TOTAL_MAX - total;
    if (remainingBudget <= 1024) {
      // Меньше 1 KB остатка — больше не имеет смысла читать
      break;
    }
    const perFileCap = Math.min(PER_FILE_MAX, remainingBudget);
    const truncated = raw.length > perFileCap;
    const text = truncated
      ? raw.slice(0, perFileCap) + "\n… [обрезано]"
      : raw;
    total += text.length;
    items.push({
      id: newId(),
      filename: f.name,
      relativePath: relPath,
      sizeBytes: text.length,
      truncated,
      kind: "folder-file",
      textContent: text,
    });
  }

  // 5) Root name — первая компонента webkitRelativePath
  const firstRel =
    (taken[0] as File & { webkitRelativePath?: string })?.webkitRelativePath || "";
  const rootName = firstRel.split(/[\\/]/)[0] || "(папка)";

  const folderBundle: FolderBundle | null = items.length
    ? { rootName, itemIds: items.map((i) => i.id) }
    : null;

  return { items, folderBundle, skipped: skipped + overflowSkipped };
}

/** Форматирует размер в человекочитаемый вид. */
export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(2)} MB`;
}

/** Суммарный размер активных (text + folder-file) аттачментов. */
export function totalActiveSize(items: AttachmentItem[]): number {
  return items
    .filter((i) => i.kind !== "unsupported")
    .reduce((s, i) => s + i.sizeBytes, 0);
}

/** Валидация при добавлении: возвращает (ok=true | error message). */
export function validateAdd(
  current: AttachmentItem[],
  incoming: AttachmentItem[],
): { ok: boolean; message?: string } {
  const activeCount =
    current.filter((i) => i.kind !== "unsupported").length +
    incoming.filter((i) => i.kind !== "unsupported").length;
  if (activeCount > MAX_FILES) {
    return {
      ok: false,
      message: `Лимит ${MAX_FILES} файлов на сообщение. Лишние ${activeCount - MAX_FILES} пропущены.`,
    };
  }
  const totalSize = totalActiveSize(current) + totalActiveSize(incoming);
  if (totalSize > TOTAL_MAX) {
    return {
      ok: false,
      message: `Лимит 1 MB суммарно. Сейчас ${formatBytes(totalSize)} — лишнее обрезано.`,
    };
  }
  return { ok: true };
}

/**
 * Payload-форма для отправки в Rust `send_chat_message`.
 * Поле `text_content` snake_case как ожидает serde на той стороне.
 */
export interface AttachmentPayload {
  filename: string;
  size_bytes: number;
  text_content: string;
  relative_path?: string;
}

export function toPayload(items: AttachmentItem[]): AttachmentPayload[] {
  return items
    .filter((i) => i.kind !== "unsupported" && i.textContent.length > 0)
    .map((i) => ({
      filename: i.filename,
      size_bytes: i.sizeBytes,
      text_content: i.textContent,
      relative_path: i.relativePath,
    }));
}
