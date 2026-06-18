use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerdictStatus {
    Pass,
    Fail,
    Uncertain,
}

impl fmt::Display for VerdictStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VerdictStatus::Pass => write!(f, "ГОДНО"),
            VerdictStatus::Fail => write!(f, "БРАК"),
            VerdictStatus::Uncertain => write!(f, "НЕОПРЕДЕЛЁННО"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Verdict {
    pub status: VerdictStatus,
    pub reasons: String,
}

const FAIL_PATTERNS: &[&str] = &[
    // Негативы — ловят отрицание pass-слов («НЕ ГОДЕН», «НЕ ПРИНЯТО»),
    // чтобы substring «ГОДЕН»/«ПРИНЯТ» не дал ложный Pass. Проверяются ПЕРВЫМИ.
    "НЕ ГОД",
    "НЕГОД",
    "НЕ ПРИН",
    "НЕ ОДОБР",
    "НЕ УТВЕРЖД",
    "НЕ КОРРЕКТ",
    "НЕВЕРН",
    "БРАК",
    "ДОРАБОТК",
    "ОШИБК",
    "НЕСООТВЕТСТВ",
    "ОТКЛОНЕН",
    "НЕ ИСПРАВЛЕН",
    "НЕ РАБОТА",
    "НЕ РЕАЛИЗОВАН",
    "НЕ ВЫПОЛНЕН",
    "НЕДОРАБОТ",
    "ДЕФЕКТ",
    "НЕТ РЕЗУЛЬТАТ",
    "НЕТ ОТВЕТ",
    "НЕТ ДАНН",
    "ПРОВАЛ",
    "НЕПРИЕМЛЕМ",
];

const PASS_PATTERNS: &[&str] = &[
    "ГОДНО",
    "ГОДЕН",
    "ПРИНЯТ",
    "ОДОБРЕН",
    "УТВЕРЖДЕН",
    "КОРРЕКТН",
];

fn detect_fail(upper: &str) -> bool {
    FAIL_PATTERNS.iter().any(|p| upper.contains(p))
}

fn detect_pass(upper: &str) -> bool {
    PASS_PATTERNS.iter().any(|p| upper.contains(p))
}

/// Parse verdict.md content into a structured Verdict.
///
/// Phase 1: explicit markers on first meaningful line (`ГОДНО`/`БРАК`/`ДОРАБОТК`).
/// Phase 2 (fallback): deep scan of full text for fail/pass patterns.
/// Fail always takes priority over pass.
pub fn parse_verdict(content: &str) -> Verdict {
    let mut lines = content.lines().peekable();

    let first_line = loop {
        match lines.next() {
            Some(l) if l.trim().is_empty() => continue,
            Some(l) => break l.trim().to_string(),
            None => {
                return Verdict {
                    status: VerdictStatus::Uncertain,
                    reasons: "verdict.md пуст".to_string(),
                };
            }
        }
    };

    let reasons: String = lines
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();

    let upper_first = first_line.to_uppercase();
    let upper_full = content.to_uppercase();

    // FAIL имеет АБСОЛЮТНЫЙ приоритет и проверяется ПЕРВЫМ — на первой строке,
    // затем по всему тексту. FAIL_PATTERNS включает негативы («НЕ ГОД», «НЕ ПРИН»),
    // поэтому «НЕ ГОДЕН»/«НЕ ПРИНЯТО» НЕ дадут ложный Pass через подстроку.
    let status = if detect_fail(&upper_first) {
        VerdictStatus::Fail
    } else if detect_pass(&upper_first) {
        VerdictStatus::Pass
    } else if detect_fail(&upper_full) {
        VerdictStatus::Fail
    } else if detect_pass(&upper_full) {
        VerdictStatus::Pass
    } else {
        VerdictStatus::Uncertain
    };

    Verdict { status, reasons }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pass_simple() {
        let v = parse_verdict("ВЕРДИКТ: ГОДНО\nВсё отлично.");
        assert_eq!(v.status, VerdictStatus::Pass);
        assert_eq!(v.reasons, "Всё отлично.");
    }

    #[test]
    fn pass_goden_form() {
        let v = parse_verdict("ВЕРДИКТ: ГОДЕН\nБез замечаний.");
        assert_eq!(v.status, VerdictStatus::Pass);
    }

    #[test]
    fn fail_brak() {
        let v = parse_verdict("ВЕРДИКТ: БРАК\n1. Нет обработки ошибок\n2. Нет тестов");
        assert_eq!(v.status, VerdictStatus::Fail);
        assert!(v.reasons.contains("Нет обработки ошибок"));
        assert!(v.reasons.contains("Нет тестов"));
    }

    #[test]
    fn fail_na_dorabotku() {
        let v = parse_verdict("ВЕРДИКТ: НА ДОРАБОТКУ\nСлабая реализация.");
        assert_eq!(v.status, VerdictStatus::Fail);
        assert_eq!(v.reasons, "Слабая реализация.");
    }

    #[test]
    fn fail_takes_priority_over_pass() {
        let v = parse_verdict("ВЕРДИКТ: ГОДНО НА ДОРАБОТКУ");
        assert_eq!(v.status, VerdictStatus::Fail, "БРАК/ДОРАБОТКУ wins over ГОДНО");
    }

    #[test]
    fn uncertain_garbage() {
        let v = parse_verdict("Какой-то мусор\nБез структуры");
        assert_eq!(v.status, VerdictStatus::Uncertain);
        assert_eq!(v.reasons, "Без структуры");
    }

    #[test]
    fn uncertain_empty() {
        let v = parse_verdict("");
        assert_eq!(v.status, VerdictStatus::Uncertain);
        assert!(v.reasons.contains("пуст"));
    }

    #[test]
    fn uncertain_only_whitespace() {
        let v = parse_verdict("   \n   \n  ");
        assert_eq!(v.status, VerdictStatus::Uncertain);
    }

    #[test]
    fn leading_blank_lines_skipped() {
        let v = parse_verdict("\n\n  ВЕРДИКТ: ГОДНО  \nПримечание.");
        assert_eq!(v.status, VerdictStatus::Pass);
        assert_eq!(v.reasons, "Примечание.");
    }

    #[test]
    fn case_insensitive() {
        let v = parse_verdict("вердикт: брак\nПричина: ошибки.");
        assert_eq!(v.status, VerdictStatus::Fail);
        assert_eq!(v.reasons, "Причина: ошибки.");
    }

    #[test]
    fn multiline_reasons() {
        let v = parse_verdict(
            "ВЕРДИКТ: БРАК\n\
             1. Не хватает валидации входных данных\n\
             2. SQL-инъекция в поле name\n\
             3. Нет unit-тестов",
        );
        assert_eq!(v.status, VerdictStatus::Fail);
        let lines: Vec<_> = v.reasons.lines().collect();
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn display_trait() {
        assert_eq!(VerdictStatus::Pass.to_string(), "ГОДНО");
        assert_eq!(VerdictStatus::Fail.to_string(), "БРАК");
        assert_eq!(VerdictStatus::Uncertain.to_string(), "НЕОПРЕДЕЛЁННО");
    }

    // ── Deep scan tests ──────────────────────────────────────────────────

    #[test]
    fn deep_fail_oshibka() {
        let v = parse_verdict("Результат проверки:\nОшибка: функция не обрабатывает NULL");
        assert_eq!(v.status, VerdictStatus::Fail);
    }

    #[test]
    fn deep_fail_ne_ispravleno() {
        let v = parse_verdict("Замечания по коду:\nПроблема не исправлена, баг остаётся.");
        assert_eq!(v.status, VerdictStatus::Fail);
    }

    #[test]
    fn deep_fail_net_rezultata() {
        let v = parse_verdict("Итог:\nНет результата, файл пуст.");
        assert_eq!(v.status, VerdictStatus::Fail);
    }

    #[test]
    fn deep_fail_nesootvetstvie() {
        let v = parse_verdict("Анализ:\nНесоответствие ТЗ и реализации.");
        assert_eq!(v.status, VerdictStatus::Fail);
    }

    #[test]
    fn deep_fail_ne_rabotaet() {
        let v = parse_verdict("Проверка:\nМодуль не работает при нагрузке.");
        assert_eq!(v.status, VerdictStatus::Fail);
    }

    #[test]
    fn deep_fail_otklonenо() {
        let v = parse_verdict("Решение:\nЗадача отклонена контролёром.");
        assert_eq!(v.status, VerdictStatus::Fail);
    }

    #[test]
    fn deep_fail_defekt() {
        let v = parse_verdict("Контроль качества:\nОбнаружен дефект в логике расчёта.");
        assert_eq!(v.status, VerdictStatus::Fail);
    }

    #[test]
    fn deep_pass_prinyato() {
        let v = parse_verdict("Результат:\nВсё принято, работа выполнена.");
        assert_eq!(v.status, VerdictStatus::Pass);
    }

    #[test]
    fn deep_pass_odobreno() {
        let v = parse_verdict("Итог:\nОдобрено, можно деплоить.");
        assert_eq!(v.status, VerdictStatus::Pass);
    }

    #[test]
    fn deep_pass_korrektnо() {
        let v = parse_verdict("Проверка:\nРеализация корректна.");
        assert_eq!(v.status, VerdictStatus::Pass);
    }

    #[test]
    fn deep_fail_beats_deep_pass() {
        let v = parse_verdict("Оценка:\nЧасть одобрена, но обнаружен дефект.");
        assert_eq!(v.status, VerdictStatus::Fail, "fail patterns override pass");
    }

    #[test]
    fn explicit_pass_not_overridden_by_deep() {
        let v = parse_verdict("ВЕРДИКТ: ГОДНО\nБез замечаний, ошибок нет.");
        assert_eq!(v.status, VerdictStatus::Pass, "explicit ГОДНО on first line wins");
    }

    // ── Негативы: ложный Pass НЕ должен возникать (Cursor REJECT блокер) ──

    #[test]
    fn fail_ne_goden() {
        let v = parse_verdict("ВЕРДИКТ: НЕ ГОДЕН\nЕсть проблемы.");
        assert_eq!(v.status, VerdictStatus::Fail, "«НЕ ГОДЕН» = брак, не Pass");
    }

    #[test]
    fn fail_ne_godno() {
        let v = parse_verdict("ВЕРДИКТ: НЕ ГОДНО");
        assert_eq!(v.status, VerdictStatus::Fail);
    }

    #[test]
    fn fail_negoden_slitno() {
        let v = parse_verdict("ВЕРДИКТ: НЕГОДЕН");
        assert_eq!(v.status, VerdictStatus::Fail);
    }

    #[test]
    fn fail_ne_prinyato() {
        let v = parse_verdict("ВЕРДИКТ: НЕ ПРИНЯТО\nДоработать.");
        assert_eq!(v.status, VerdictStatus::Fail, "«НЕ ПРИНЯТО» = брак, не Pass");
    }

    #[test]
    fn deep_fail_ne_goden() {
        let v = parse_verdict("Оценка работы:\nРабота не годна к сдаче.");
        assert_eq!(v.status, VerdictStatus::Fail);
    }
}
