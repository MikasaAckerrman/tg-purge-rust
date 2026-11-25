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
    // Считаем ДО цикла: for (name, proof, ok) in rows ПОТРЕБЛЯЕТ Vec
    // (раунд 10: E0382 «borrow of moved value: rows»).
    let total = rows.len();
    let missing = rows.iter().filter(|(_, _, ok)| !*ok).count();
    let sep = "-".repeat(112);
    println!("{:<44} {:<58} {}", "ВОЗМОЖНОСТЬ", "ЧЕМ ДОКАЗАНО", "ЕСТЬ");
    println!("{sep}");
    for (name, proof, ok) in rows {
        println!(
            "{:<44} {:<58} {}",
            name,
            proof,
            if ok { "да" } else { "НЕТ — гейт не пройден" }
        );
    }
    // ★ println!("-".repeat(..)) — format-строка обязана быть литералом
    //   (раунд 10: «expected `,`, found .» + «argument never used»).
    println!("{sep}");
    println!(
        "итого: {} из {} доступно; непройденных гейтов: {}",
        total - missing,
        total,
        missing
    );
    let _ = std::io::stdout().flush();
}
