# Phase 1 SPEC — Iteration A

AgentPod Phase 1 (Service Bureau + PAL + Pod Runtime model switching) — комплект SPEC-документов Итерации A.

Источник: in-app Vault (`%APPDATA%\ru.msproltd.corp\Vault\03-Phases\`). Эта копия в репозитории — snapshot на момент завершения Итерации A (2026-05-28).

## Состав (9 документов)

| Документ | Версия | Автор | Назначение |
|---|---|---|---|
| [phase-1-pal-trait-spec.md](phase-1-pal-trait-spec.md) | v3 | Гендир + Cursor | Контракт `PostRuntimeProvider` trait + типы (ProviderRequest/Response/Error, Tier, HealthStatus, Capabilities). Source of truth. |
| [phase-1-claude-cli-driver-IMPL-REFERENCE.md](phase-1-claude-cli-driver-IMPL-REFERENCE.md) | v1.1 | программист | Реализация trait для Claude CLI (subprocess `--print --agent`). По реальному `post_executor.rs`. |
| [phase-1-qwen-http-driver-IMPL-REFERENCE.md](phase-1-qwen-http-driver-IMPL-REFERENCE.md) | v1.1 | программист | Реализация trait для Qwen (HTTP OpenAI-compat SSE). По реальному `qwen_bridge.rs`. |
| [phase-1-external-gateway-driver-IMPL-REFERENCE.md](phase-1-external-gateway-driver-IMPL-REFERENCE.md) | v1 | программист | Stub-драйвер (NotImplemented в Phase 1; Phase 2 reuse WS gateway 8899). |
| [phase-1-definition-of-done.md](phase-1-definition-of-done.md) | v1.1 | Гендир + программист | 43 проверяемых критерия (Acceptance / Functional / Regression / Quality / Deliverables) + scope + sequencing. |
| [phase-1-risk-register.md](phase-1-risk-register.md) | v1.1 | программист + Гендир | 15 технических рисков (P×I + mitigation) + бизнес/процессные риски. |
| [phase-1-ui-wireframes-spec.md](phase-1-ui-wireframes-spec.md) | v1.1 | Гендир + Cursor | Wireframes Service Bureau + Pod Runtime model switcher. |
| [phase-1-ui-design-language-reference.md](phase-1-ui-design-language-reference.md) | — | Гендир | UI design language reference (цвета, паттерны компонентов). |
| [phase-1-current-db-schema.sql](phase-1-current-db-schema.sql) | v1.0.33 | программист/Cursor | Актуальная схема БД (13 таблиц) — базис для миграций 08-09. |

## Методология

Документы созданы по 4-столповой методологии (см. Vault `02-Patterns/playbook-итерации-spec-роли-investigation-verify.md`):
1. Разделение ролей (Гендир = архитектура/scope; программист = технический inventory из реального кода; Cursor = независимый verify; Владелец = approve).
2. Investigation перед проектированием.
3. Real-code-paths (не из памяти).
4. Cursor-verify обязателен для каждого артефакта.

## Не включено в snapshot (по причинам)

- `phase-1-backlog.md` — живой рабочий документ (BL-P1-001…005), не финальный SPEC.
- `phase-1-current-db-tables.md` — устарел (описывает v1.0.32), заменён `phase-1-current-db-schema.sql`.
- `phase-1-claude-cli-driver-spec.md` — skeleton Гендира v1, **заменён** IMPL-REFERENCE (решение Владельца «Вариант B»: кто видит реальный код — тот и автор reference).

## Статус

**Итерация A — комплект SPEC готов.** Следующее — Итерация B (implementation): миграции 08-09 → PAL trait + драйверы + orchestrator → post_executor integration → Service Bureau UI → MSI 1.0.34. Порядок — см. `phase-1-definition-of-done.md` §«Implementation sequencing».
