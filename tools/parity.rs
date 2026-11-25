//! Паритет Rust ↔ Python по чистой логике лимитов (гейт R3/R4 из PLAN_RUST §3).
//!
//! Зачем: Python-движок (`core/model.py::next_allowed`) работает в бою уже сутки,
//! а Rust-порт только что написал я. Сверка «тесты зелёные» не доказывает, что
//! ДВЕ реализации считают одинаково — ровно на этом я и поймал баг индекса в
//! `Window::wait` (раунд 12: 50 с вместо 40 с). Здесь прогоняется матрица сценариев
//! через Rust и через Python и сравниваются числа.
//!
//! Локальный запуск БЕЗ CI и без grammers:
//!     rustc --crate-type=lib --edition 2021 src/rate.rs -o /tmp/librate.rlib
//!     rustc --edition 2021 tools/parity.rs --extern rate=/tmp/librate.rlib -o /tmp/parity
//!     /tmp/parity            # читает сценарии со stdin, печатает ожидания в мс
//!     python3 tools/parity.py
//!
//! Формат строки (stdin): `<limit> <span_s> <n> <hit_0> ... <hit_n-1> <now_s>`
//! — все времена в секундах ОТНОСИТЕЛЬНО одного base Instant, вывод — миллисекунды.

use std::io::{BufRead, Write};
use std::time::{Duration, Instant};

use rate::Window;

fn main() {
    let stdin = std::io::stdin();
    let out = std::io::stdout();
    let mut locked = out.lock();
    // Один base на весь прогон: Instant нельзя сконструировать извне, поэтому
    // все смещения считаем от него — сравнение остаётся корректным.
    let base = Instant::now();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(s) => s,
            Err(_) => break,
        };
        let toks: Vec<&str> = line.split_whitespace().collect();
        if toks.len() < 4 {
            continue;
        }
        let nums: Vec<i64> = match toks.iter().map(|t| t.parse::<i64>()).collect::<Result<_, _>>() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let limit = nums[0].max(0) as usize;
        let span_s = nums[1].max(0) as u64;
        let n = nums[2].max(0) as usize;
        let hits: Vec<i64> = nums[3..3 + n].to_vec();
        let now_off = nums[3 + n];
        let mut w = Window::new(limit, Duration::from_secs(span_s));
        for h in hits {
            w.record(at(base, h));
        }
        let wait = w.wait(at(base, now_off));
        let _ = writeln!(locked, "{}", wait.as_millis());
    }
}

fn at(base: Instant, secs: i64) -> Instant {
    if secs < 0 {
        base - Duration::from_secs((-secs) as u64)
    } else {
        base + Duration::from_secs(secs as u64)
    }
}
