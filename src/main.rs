//! TGPURGE Rust-движок: ГЕЙТ R1/R3 (сборка + компиляционные доказательства).
//!
//! Пока это НЕ боевой движок: цель модулей `caps` — за 2 минуты CI-прогона получить
//! ответ, какие операции grammers 0.12 реально даёт, а какие нет. Описание
//! супергруппы (`channels::EditDescription`) в опубликованном слое 233 ОТСУТСТВУЕТ —
//! это единственный найденный провал, и он виден ДО написания движка.

mod caps;

use std::io::Write;

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_default();
    if arg == "--caps" {
        print_caps();
        return;
    }
    // Без аргументов — только проверка, что типы сходятся (сети нет).
    caps::pool_typechecks(0);
    println!("purge spike: компиляция типов прошла; запускай с --caps для списка возможностей");
}

fn print_caps() {
    let rows = caps::capabilities();
    println!("{:<44} {:<58} {}", "ВОЗМОЖНОСТЬ", "ЧЕМ ДОКАЗАНО", "ЕСТЬ");
    println!("{}", "-".repeat(112));
    for (name, proof, ok) in rows {
        println!(
            "{:<44} {:<58} {}",
            name,
            proof,
            if ok { "да" } else { "НЕТ — гейт не пройден" }
        );
    }
    let missing = rows.iter().filter(|(_, _, ok)| !*ok).count();
    println!("-".repeat(112).as_str());
    println!(
        "итого: {} из {} доступно; непройденных гейтов: {}",
        rows.len() - missing,
        rows.len(),
        missing
    );
    let _ = std::io::stdout().flush();
}
