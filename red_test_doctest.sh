#!/bin/sh
# Красный тест сканера doctest: на ЗАВЕДОМО сломанном входе, в отдельном каталоге,
# не трогая рабочие исходники. Ассерты, которые не умеют краснеть, — не проверки
# (я на этом уже попадался трижды: tests/ast_check, pipe-валидатор, fmt_lines).
set -u
R=/var/minis/shared/rustspike
D=/tmp/doctest_red
rm -rf $D && mkdir -p $D

# 1) ровно та ловушка, что уронила CI в раунде 16 (util.rs:3-4)
cat > $D/trap.rs <<'EOF'
//! Утилиты:
//!
//!     rustc --test --edition 2021 src/util.rs -o /tmp/util && /tmp/util
//!
//! ок
fn main() {}
EOF

# 2) легальный вариант: тот же текст, но во фенсе с языком
cat > $D/ok_fenced.rs <<'EOF'
//! Утилиты:
//!
//! ```text
//! rustc --test --edition 2021 src/util.rs -o /tmp/util && /tmp/util
//! ```
fn main() {}
EOF

# 3) легальный вариант: команда сразу после строки с `:` (ленивое продолжение
#    абзаца — именно поэтому в раунде 16 упал ТОЛЬКО util.rs, а не rate.rs)
cat > $D/ok_lazy.rs <<'EOF'
//! Гонять локально:
//!     rustc --test --edition 2021 src/rate.rs -o /tmp/rate && /tmp/rate
//! Так эволюционируют за секунды.
fn main() {}
EOF

# 4) блок ВНУТРИ ``` без языка — тоже doctest, обязан быть пойман
cat > $D/trap_bare_fence.rs <<'EOF'
//! Пример:
//!
//! ```
//! let x = 1;
//! rustc --version | head -1
//! ```
fn main() {}
EOF

echo "=== 1) ловушка CI-раунда-16 (ожидаем ПЛОХО) ==="
python3 -u $R/doctest_scan.py $D; echo "rc=$? (ожидаем 1)"

echo; echo "=== 2) только легальные файлы (ожидаем OK) ==="
mkdir -p $D/good && mv $D/ok_fenced.rs $D/ok_lazy.rs $D/good/
python3 -u $R/doctest_scan.py $D/good; echo "rc=$? (ожидаем 0)"

echo; echo "=== 3) рабочий каталог src/ (ожидаем OK) ==="
python3 -u $R/doctest_scan.py $R/src; echo "rc=$? (ожидаем 0)"
