# Первичные тексты восьми Linux crates — 2026-11-11

База: `565fa26bd745565dbe64a47af8d970b6e57a3073`.
Scope: Linux source inventory, без Android, Rust-сборки и изменения зависимостей.

У clang-format 0.3.0, codespan-reporting 0.11.1, cxx-qt/build/gen/lib/macro
0.10.0 и qt-build-utils 0.10.0 в shipped crate отсутствуют первичные тексты.
Commit взят из `.cargo_vcs_info.json`, repository из shipped Cargo.toml,
registry source/checksum из Cargo.lock. Получены официальные GitHub tree и raw
файлы строго на этих commits:

- KDAB/clang-format-rs: `82e49611760ce781e3753dde57975ba576b286c3`, LICENSES/MIT.txt и Apache-2.0.txt.
- brendanzab/codespan: `fd389a13f5bb6d625b71e2e4694b26e127f393f9`, LICENSE (Apache 2.0).
- KDAB/cxx-qt: `61562797ba25558a7c7dba8017deca038f3ee9e4`, LICENSES/MIT.txt и Apache-2.0.txt.

Полные URLs и SHA-256 каждого текста, identity и path_in_vcs каждого crate:
[manifest](../../packaging/licenses/vendor/sources.json). Все загруженные байты
дополнительно сверены по Git blob SHA-1 официальных pinned trees. SPDX не
использован для генерации текста или выбора ветви OR. CRLF и пробелы сохранены.

Fallback применяется только при отсутствии найденных package license files.
Перед чтением проверяются точные name/version/source, lock checksum и VCS commit.
Имя текста ограничено одним компонентом vendor; чтение через существующую
FD-relative NOFOLLOW границу, regular file, лимит 4 MiB, непустой текст и SHA-256.
Manifest включён в provenance report. Непроверенный report не управляет I/O.

Behavioral RED: новый тест падает на отсутствии curated_license.
GREEN: 13 targeted tests, включая прежние 12; новый тест проверяет mismatch
version/source/checksum/commit/hash/path, пустой/пробельный текст, symlink,
интеграционные output/check и побайтовое сохранение CRLF/пробелов.
Штатный tools/tests discovery: 145 PASS. Scoped Ruff check/format и diff check PASS.
Реальные output и check: exit 0, 225 packages, errors=0. Команды выполняются через обязательный
with-rust-slot, jobs1 и отдельный transient CARGO_TARGET_DIR с flock корня.

Transient root пуст: Rust target/build не создавались; после завершения
metadata-процессов каталог удалён через rmdir, cargo clean неприменим.

Логи вне TTL: `/home/myrs/projects/yonik/local/validation/linux-curated-licenses/`.
ELF/Qt/Go composition остаётся unknown. Это закрытие восьми missing_license
в source inventory, не юридическая сертификация, полный P25 PASS или релиз.
