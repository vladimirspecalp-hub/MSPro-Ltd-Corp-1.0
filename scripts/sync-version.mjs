#!/usr/bin/env node
/**
 * sync-version.mjs — синхронизирует версию во всех файлах проекта.
 *
 * Usage:
 *   node scripts/sync-version.mjs v1.0.1
 *   node scripts/sync-version.mjs 1.0.1
 *
 * Updates:
 *   - src-tauri/tauri.conf.json   (поле "version")
 *   - src-tauri/Cargo.toml        (поле version в [package])
 *   - package.json                (root)
 *
 * Используется CI (.github/workflows/tauri-release.yml) перед сборкой,
 * чтобы все артефакты (MSI, latest.json) имели одну версию,
 * соответствующую тегу.
 */

import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { join, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(__dirname, "..");

function parseVersion(input) {
  if (!input) {
    throw new Error("version argument is required (e.g. v1.0.1 or 1.0.1)");
  }
  const cleaned = String(input).trim().replace(/^v/i, "");
  if (!/^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.\-]+)?$/.test(cleaned)) {
    throw new Error(
      `invalid semver: "${input}" (expected major.minor.patch[-pre][+meta])`
    );
  }
  return cleaned;
}

function updateJsonVersion(relPath, version) {
  const full = join(ROOT, relPath);
  if (!existsSync(full)) {
    console.warn(`[skip] ${relPath} — not found`);
    return false;
  }
  const raw = readFileSync(full, "utf8");
  const data = JSON.parse(raw);
  const before = data.version;
  data.version = version;
  const trailingNl = raw.endsWith("\n") ? "\n" : "";
  writeFileSync(full, JSON.stringify(data, null, 2) + trailingNl);
  console.log(`[ok]   ${relPath}: ${before ?? "(absent)"} -> ${version}`);
  return true;
}

function updateCargoTomlVersion(relPath, version) {
  const full = join(ROOT, relPath);
  if (!existsSync(full)) {
    console.warn(`[skip] ${relPath} — not found`);
    return false;
  }
  const text = readFileSync(full, "utf8");
  const pkgRe = /(\[package\][\s\S]*?\n)version\s*=\s*"[^"]*"/m;
  if (!pkgRe.test(text)) {
    throw new Error(`${relPath}: cannot find [package].version`);
  }
  const before = (text.match(/\[package\][\s\S]*?\nversion\s*=\s*"([^"]*)"/) || [])[1];
  const next = text.replace(pkgRe, (_m, head) => `${head}version = "${version}"`);
  writeFileSync(full, next);
  console.log(`[ok]   ${relPath}: ${before ?? "(absent)"} -> ${version}`);
  return true;
}

function main() {
  const arg = process.argv[2];
  const version = parseVersion(arg);
  console.log(`Syncing project version -> ${version}`);

  const targets = [
    () => updateJsonVersion("src-tauri/tauri.conf.json", version),
    () => updateCargoTomlVersion("src-tauri/Cargo.toml", version),
    () => updateJsonVersion("package.json", version),
  ];

  let updated = 0;
  for (const fn of targets) {
    if (fn()) updated += 1;
  }
  console.log(`Done: ${updated}/${targets.length} files updated.`);
  if (updated === 0) {
    process.exit(1);
  }
}

try {
  main();
} catch (err) {
  console.error(`sync-version: ${err.message}`);
  process.exit(1);
}
