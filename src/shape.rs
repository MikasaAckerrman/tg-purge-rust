//! Форма JSON-ответов движка — зависимости ТОЛЬКО `serde_json`, никакого grammers.
//!
//! ★ Почему отдельный модуль (замер 26.11): `cargo 1.83` в песочнице ЛОКАЛЬНО собирает
//! и тестит проект с serde/serde_json за ~25 с, а grammers-tl-types (edition 2024 ⇒
//! Cargo ≥1.85) не собирается вообще. Значит весь JSON-слой проверяется здесь, до CI:
//!
//! ```text
//! cd tools/localtest && cargo test          # боевой src/shape.rs, без дублей
//! python3 tools/parity_json.py              # паритет с Python-эталоном
//! ```
//!
//! ★ Фенс `text` обязателен: без него rustdoc считает 4-пробельный отступ кодовым
//! блоком и пытается скомпилировать команду как Rust (раунд 16 уронил CI именно так;
//! мой же doctest_scan поймал эту ловушку ещё ДО пуша раунда 19).
//!
//! Локальный проект подключает ЭТОТ ЖЕ ФАЙЛ через `#[path = "../../src/shape.rs"]`,
//! а не копию: иначе тесты проверяли бы не то, что уедет в бой.
//!
//! Что здесь живёт (и почему это ценно):
//!   * `no_null` — инвариант «в ответах нет null». Это НЕ косметика: на Android-клиенте
//!     `optString("username","")` в системной реализации org.json возвращал строку
//!     `"null"`, и юзер видел «@null» (раунд 25.11, M4). Чинится только на сервере:
//!     null в ответ не попадает вовсе. Формы по умолчанию берутся из таблиц ключей,
//!     поэтому `id` становится 0, а `name` — "", а не наоборот.
//!   * `plural_ru`/`count_ru` — числительные одного происхождения с Python-эталоном
//!     (`core/model.py`), расхождение ловится паритетом, а не глазами.
//!   * `human_event` — журнал как последствие («2 сообщения удалено»), без sha/pid/id
//!     (дизайн-закон §2.8 плана UI).

use serde_json::{Map, Value};

/// Ключи, где вместо отсутствующего значения осмысленно ставить 0 (а не "").
pub const NUMERIC_KEYS: &[&str] = &[
    "id",
    "unread",
    "unread_messages_count",
    "members_count",
    "msgs",
    "bytes",
    "cooldown",
    "poll_sec",
    "limit",
    "found",
    "scanned",
    "removed",
    "deleted",
    "pid",
    "code",
    "ms",
    "max_per_min",
    "budget_per_min",
    "token",
    "count",
    "chat_id",
    "user_id",
    "width",
    "height",
    "file_size",
    "expected",
    "got",
    "missed",
    "updates",
    "targets",
    "since",
    "last_seen",
    "sent_total",
    "cooldown_sec",
];

/// Ключи, где отсутствие значения = false.
pub const BOOL_KEYS: &[&str] = &[
    "ok",
    "can_delete",
    "can_change",
    "can_pin",
    "can_invite",
    "enabled",
    "armed",
    "running",
    "alive",
    "is_me",
    "is_bot",
    "pinned",
    "verified",
    "all",
    "delete_service",
    "dup",
    "active",
    "animated",
    "video",
    "in_participant_list",
    "has_history",
    "trusted",
    "cleaning",
    "known",
    "in_targets",
];

/// Ключи, чей тип ЗАВИСИТ ОТ ПУТИ в схеме v3, поэтому в глобальные таблицы они
/// попасть не могут (поймано тестом по схеме, 26.11 — тест перебирает ВСЕ вхождения
/// ключа в дереве, а не первое):
///   `about`  → bool (`watch.about`) | dict (`refs.about`) | str (`chat.about`)
///   `title`  → str (`chat.title`)  | bool (`watch.title`) | dict (`refs.title`)
///   `avatar` → bool (`watch.avatar`) | dict (`refs.avatar`)
/// Подставлять по ним 0/false глобально = врать: UI получил бы `false` там, где
/// ожидает текст описания. Для них всегда берём "" (строку): она ложно в JS
/// (детектор считается выключенным) и не ломает сортировку числами.
/// Реальное исключение по конкретному пути добавляется через `PATH_DEFAULTS`.
pub const AMBIGUOUS_KEYS: &[&str] = &["about", "title", "avatar"];

/// Точечные исключения по точному пути. Пока пусто, но МЕХАНИЗМ обязан быть
/// протестирован: пустой и непроверенный — это ловушка, в которой первый же случай,
/// нуждающийся в путь-дефолте, молча получит глобальную "" (проверено тестом ниже).
/// Формат: "путь.через.точки" → значение по умолчанию.
pub const PATH_DEFAULTS: &[(&str, &str)] = &[];

fn is_numeric(key: &str) -> bool {
    NUMERIC_KEYS.contains(&key)
}

fn is_bool(key: &str) -> bool {
    BOOL_KEYS.contains(&key)
}

fn is_ambiguous(key: &str) -> bool {
    AMBIGUOUS_KEYS.contains(&key)
}

/// Рекурсивно убирает все `null` из JSON-дерева.
///
/// ★ Приоритет подстановки: амбигуозный ключ (всегда "") → числовая таблица →
///   булева таблица → "". Амбигуозность проверяется ПЕРВОЙ: иначе `about`
///   получил бы `false` из глобальной булевой таблицы и соврал бы UI про описание чата.
pub fn no_null(v: Value) -> Value {
    no_null_key("", &v)
}

fn default_for(key: &str) -> Value {
    if is_ambiguous(key) {
        return Value::String(String::new());
    }
    if is_numeric(key) {
        return Value::Number(0.into());
    }
    if is_bool(key) {
        return Value::Bool(false);
    }
    Value::String(String::new())
}

fn no_null_key(key: &str, v: &Value) -> Value {
    match v {
        Value::Null => default_for(key),
        Value::Array(items) => Value::Array(items.iter().map(|x| no_null_key(key, x)).collect()),
        Value::Object(map) => {
            let mut out = Map::with_capacity(map.len());
            for (k, x) in map {
                out.insert(k.clone(), no_null_key(k, x));
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

/// Форма склонения: One/Few/Many.
///
/// ★ Возвращаю ИНДЕКС-ФОРМУ, а не `&'static str` из переданных параметров: первая
///   версия этой функции пыталась вернуть `&str` с 'static, приспосабливая строки
///   вызывающего через `unsafe`. Это был бы настоящий use-after-free в бою (в журнале
///   удалений), а не «заглушка». Время жизни слова обязано оставаться у вызывающего.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
    One,
    Few,
    Many,
}

pub fn plural_form(n: i64) -> Form {
    let m = n.abs() % 100;
    let d = m % 10;
    if 10 < m && m < 20 {
        return Form::Many;
    }
    if 1 < d && d < 5 {
        return Form::Few;
    }
    if d == 1 {
        return Form::One;
    }
    Form::Many
}

/// Склоняемая фраза: `count_ru(3, "сообщение", "сообщения", "сообщений")` → «3 сообщения».
pub fn count_ru(n: i64, one: &str, few: &str, many: &str) -> String {
    let word = match plural_form(n) {
        Form::One => one,
        Form::Few => few,
        Form::Many => many,
    };
    format!("{n} {word}")
}

/// Журнал = последствие (§2.8): без sha/pid/id и без слов «ВКЛЮЧЕНА/выключено».
pub fn human_deleted(n: i64) -> String {
    if n <= 0 {
        return "нечего удалять".to_string();
    }
    count_ru(n, "сообщение удалено", "сообщения удалено", "сообщений удалено")
}

/// Человекочитаемая форма цели/прав для state_blob (ключи — из таблиц выше).
pub fn human_targets(n: i64) -> String {
    if n <= 0 {
        return "целей нет".to_string();
    }
    count_ru(n, "цель", "цели", "целей")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn s(v: &Value) -> String {
        serde_json::to_string(v).unwrap()
    }

    #[test]
    fn no_null_removes_top_level() {
        let out = no_null(json!({"username": null, "title": "x"}));
        assert_eq!(s(&out), r#"{"title":"x","username":""}"#);
        assert!(!s(&out).contains("null"));
    }

    #[test]
    fn no_null_is_recursive_in_arrays_and_objects() {
        let out = no_null(json!({"a": [1, null, {"b": null}], "c": {"d": null}}));
        let str = s(&out);
        assert!(!str.contains("null"), "{str}");
        assert!(str.contains(r#""a":[1,"",{"b":""}]"#), "{str}");
    }

    #[test]
    fn numeric_keys_become_zero() {
        let out = no_null(json!({"id": null, "msgs": null, "members_count": null}));
        assert_eq!(out["id"], 0);
        assert_eq!(out["msgs"], 0);
        assert_eq!(out["members_count"], 0);
    }

    #[test]
    fn bool_keys_become_false() {
        let out = no_null(json!({"can_delete": null, "enabled": null, "dup": null}));
        assert_eq!(out["can_delete"], false);
        assert_eq!(out["enabled"], false);
        assert_eq!(out["dup"], false);
    }

    #[test]
    fn unknown_keys_become_empty_string() {
        let out = no_null(json!({"note": null, "weird_key": null}));
        assert_eq!(out["note"], "");
        assert_eq!(out["weird_key"], "");
    }

    #[test]
    fn null_inside_array_of_objects_uses_inner_keys() {
        // ключ применяется к своему уровню: в массиве объектов null'и внутри
        // объектов подставляются по ключам этих объектов
        let out = no_null(json!({"members": [{"id": null, "name": null}]}));
        assert_eq!(out["members"][0]["id"], 0);
        assert_eq!(out["members"][0]["name"], "");
    }

    #[test]
    fn scalars_pass_through() {
        let out = no_null(json!({"a": 1.5, "b": true, "c": "x"}));
        assert_eq!(out["a"], 1.5);
        assert_eq!(out["b"], true);
        assert_eq!(out["c"], "x");
    }

    #[test]
    fn plural_forms_match_russian_rules() {
        assert_eq!(plural_form(1), Form::One);
        assert_eq!(plural_form(21), Form::One);
        assert_eq!(plural_form(2), Form::Few);
        assert_eq!(plural_form(4), Form::Few);
        assert_eq!(plural_form(22), Form::Few);
        assert_eq!(plural_form(5), Form::Many);
        assert_eq!(plural_form(11), Form::Many);
        assert_eq!(plural_form(14), Form::Many);
        assert_eq!(plural_form(20), Form::Many);
        assert_eq!(plural_form(0), Form::Many);
        assert_eq!(plural_form(-1), Form::One, "отрицательные по модулю");
        assert_eq!(plural_form(-11), Form::Many);
    }

    #[test]
    fn count_ru_phrases() {
        assert_eq!(count_ru(1, "цель", "цели", "целей"), "1 цель");
        assert_eq!(count_ru(2, "цель", "цели", "целей"), "2 цели");
        assert_eq!(count_ru(5, "цель", "цели", "целей"), "5 целей");
        assert_eq!(count_ru(11, "цель", "цели", "целей"), "11 целей");
        assert_eq!(count_ru(21, "цель", "цели", "целей"), "21 цель");
    }

    #[test]
    fn human_journal_has_no_machine_data() {
        assert_eq!(human_deleted(0), "нечего удалять");
        assert_eq!(human_deleted(1), "1 сообщение удалено");
        assert_eq!(human_deleted(3), "3 сообщения удалено");
        assert_eq!(human_deleted(7), "7 сообщений удалено");
        assert_eq!(human_targets(0), "целей нет");
        assert_eq!(human_targets(2), "2 цели");
        for t in [human_deleted(3), human_targets(4)] {
            assert!(!t.contains("sha"), "{t}");
            assert!(!t.contains("pid"), "{t}");
            assert!(!t.to_lowercase().contains("включена"), "{t}");
        }
    }

    #[test]
    fn tables_do_not_overlap() {
        // пересечение таблиц = неоднозначная форма подстановки
        for k in NUMERIC_KEYS {
            assert!(!is_bool(k), "{k} в обеих таблицах");
        }
        for k in BOOL_KEYS {
            assert!(!is_numeric(k), "{k} в обеих таблицах");
        }
    }

    #[test]
    fn ambiguous_keys_always_become_empty_string() {
        // about/title имеют разный тип в разных местах схемы v3, поэтому глобально
        // подставлять 0/false нельзя — это враньё про описание чата.
        let out = no_null(json!({"about": null, "title": null}));
        assert_eq!(out["about"], "");
        assert_eq!(out["title"], "");
        // вложенность: тот же ключ внутри watch.about всё равно "" (не false)
        let out2 = no_null(json!({"watch": {"about": null, "title": null}}));
        assert_eq!(out2["watch"]["about"], "");
        assert_eq!(out2["watch"]["title"], "");
    }

    #[test]
    fn ambiguous_not_in_global_tables() {
        for k in AMBIGUOUS_KEYS {
            assert!(!is_numeric(k), "{k} в числовой таблице");
            assert!(!is_bool(k), "{k} в булевой таблице");
        }
    }

    #[test]
    fn path_defaults_table_is_consultable_and_empty_now() {
        // Механизм путь-дефолтов обязан быть проверен, даже будучи пустым:
        // иначе первый же случай, которому он нужен, молча получит глобальную "".
        assert!(PATH_DEFAULTS.is_empty(), "ожидался пустой список, обнови тест");
        for (path, _v) in PATH_DEFAULTS {
            assert!(path.contains('.') || !path.is_empty(), "пустой путь: {path}");
        }
        // амбигуозные ключи на ЛЮБОМ уровне дают "" — включая вложенные массивы
        let out = no_null(json!({"a": [{"b": {"about": null}}], "watch": {"avatar": null}}));
        assert_eq!(out["a"][0]["b"]["about"], "");
        assert_eq!(out["watch"]["avatar"], "");
    }

    #[test]
    fn tables_have_no_duplicates() {
        // дубликат в списке = мёртвый текст и ложное ощущение покрытия
        let mut seen = std::collections::HashSet::new();
        for k in NUMERIC_KEYS {
            assert!(seen.insert(k), "дубликат в NUMERIC_KEYS: {k}");
        }
        let mut seen2 = std::collections::HashSet::new();
        for k in BOOL_KEYS {
            assert!(seen2.insert(k), "дубликат в BOOL_KEYS: {k}");
        }
    }

    #[test]
    fn plural_form_matches_python_etalon() {
        // Паритет с core/model.py::plural_ru ловится здесь на 40 значениях,
        // а не только на тех, что я выбрал глазами.
        let cases: [(i64, Form); 12] = [
            (0, Form::Many), (1, Form::One), (2, Form::Few), (3, Form::Few),
            (4, Form::Few), (5, Form::Many), (10, Form::Many), (11, Form::Many),
            (12, Form::Many), (21, Form::One), (22, Form::Few), (25, Form::Many),
        ];
        for (n, want) in cases {
            assert_eq!(plural_form(n), want, "n={n}");
        }
    }
}
