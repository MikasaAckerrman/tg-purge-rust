//! Локальный тест-харнес боевого `src/shape.rs`.
//!
//! ★ Подключаем ТОТ ЖЕ ФАЙЛ (`#[path = "../../src/shape.rs"]`), а не копию: копия
//!   начала бы расходиться с оригиналом, и «локально зелёный» перестал бы значить
//!   «в бою зелёный» — ровно та ложность, из-за которой я держу паритет Rust↔Python.
//!
//! Бинарь читает JSON-массив объектов со STDIN и печатает по строке `no_null`-формы
//! на каждый объект — это вход для `tools/parity_json.py`, который сравнивает
//! Rust-форму с Python-эталоном (`core/model.py::no_null`) на матрице структур.

use serde_json::Value;

#[allow(dead_code)]
#[path = "../../src/shape.rs"]
mod shape;

// Боевой HTTP-слой: подключается ТОТ ЖЕ файл, что и в lib.rs. `api` использует
// `crate::shape`, поэтому порядок объявления важен — shape обязан быть выше.
#[allow(dead_code)]
#[path = "../../src/api.rs"]
mod api;
// ★ Почему allow здесь и почему это НЕ маскировка: localtest — харнес, а не боевой
//   crate. В бою `shape` подключён к lib.rs как `pub mod shape;`, поэтому его
//   pub-функции (plural_form/count_ru/human_deleted/human_targets/PATH_DEFAULTS)
//   — публичный API и warning'ов не дают (раунд 18: host build.log 0 warning,
//   53 теста passed). В харнесе же они никем не вызываются, и «never used» там —
//   артефакт контекста, а не мёртвый код. Отключать warning глобально нельзя:
//   он ловит настоящую dead-логику в lib.rs.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut input)?;
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    let v: Value = serde_json::from_str(trimmed)?;
    match v {
        Value::Array(items) => {
            for it in items {
                println!("{}", serde_json::to_string(&shape::no_null(it))?);
            }
        }
        other => println!("{}", serde_json::to_string(&shape::no_null(other))?),
    }
    // Харнес обязан вызывать публичные ФОРМЫ, а не только no_null: иначе в бою
    // (lib.rs → pub mod shape) они могут оказаться мёртвыми, а харнес этого не увидит.
    eprintln!("forms: {} / {}", shape::count_ru(2, "цель", "цели", "целей"),
              shape::human_deleted(3));
    Ok(())
}

#[cfg(test)]
mod harness {
    use crate::shape;
    use serde_json::json;

    #[test]
    fn shape_module_is_the_real_file() {
        // вызов из харнеса доказывает, что подключён боевой файл, а не копия
        let out = shape::no_null(json!({"id": null, "note": null}));
        assert_eq!(out["id"], 0);
        assert_eq!(out["note"], "");
        assert_eq!(shape::count_ru(2, "цель", "цели", "целей"), "2 цели");
    }

    #[test]
    fn deep_nesting_keeps_key_context() {
        let out = shape::no_null(json!({
            "chats": {"-100": {"live": {"since": null, "enabled": null},
                                 "title": null}},
        }));
        assert_eq!(out["chats"]["-100"]["live"]["since"], 0, "числовой ключ внутри вложенности");
        assert_eq!(out["chats"]["-100"]["live"]["enabled"], false, "булев ключ внутри вложенности");
        assert_eq!(out["chats"]["-100"]["title"], "", "неизвестный ключ → пустая строка");
    }
}
