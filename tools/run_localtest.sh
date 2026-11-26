#!/bin/sh
# Локальный проект для тестирования БОЕВОГО src/shape.rs без grammers.
# cargo 1.83 в песочнице собирает serde/serde_json за ~25 с (замерено 26.11),
# а grammers-tl-types (edition 2024) — нет. Значит JSON-слой обязан тестироваться
# здесь, а не шестью минутами CI.
set -u
D=/var/minis/shared/rustspike/tools/localtest
cd $D || exit 1
echo "=== cargo test (боевой src/shape.rs через #[path]) ==="
if timeout 400 cargo test 2>&1 | tail -25; then :; else echo "СБОРКА/ТЕСТЫ УПАЛИ"; exit 1; fi
