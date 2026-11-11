# Bundle transport после объединения с main

Root объединил frozen PR109 `00969a1` с main `6201cbe` обычным merge.
Единственный конфликт — добавленный canonical_store module/reexport перед
inventory tests; модуль и все ранее принятые quota/OFD tests сохранены.
Helper/worker production paths побайтно не изменены относительно 00969a1;
исторический actual VM proof относится именно к frozen 00969a1, новый VM
запуск не заявляется.

Rust 1.85.1 --locked: 421 Linux lib test (31 ignored), 9 FD/source/store
integration tests — PASS. Pinned all-targets Clippy и fmt — PASS.
Source-before.json записан до проверок, после них hashes совпали.
Сохранённые lib/receiver binaries, source manifest и логи:
local/validation/pr109-main/manifest.json.

Полный P04, app AND, domain+CIDR/DNS provenance и future namespace-anchor
custody этим merge не закрываются. External FD/mount-namespace retainers
по-прежнему требуют отдельного proof; anchor не должен удерживать mnt FD
после разрешения assets без соответствующего lifetime/budget contract.
