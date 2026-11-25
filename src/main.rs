//! TGPURGE Rust-движок: ГЕЙТ R1 (сборка) + R3 (компиляционные доказательства API).
//!
//! Логика живёт в lib-крете (`purge`), чтобы один и тот же код использовали
//! и этот bin, и cdylib внутри APK. Здесь только CLI-обвязка.

use std::io::Write;

use purge::caps;

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_default();
    if arg == "--caps" {
        print_caps();
        return;
    }
    if arg == "--session-format" {
        println!("{}", purge::SESSION_FORMAT);
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
