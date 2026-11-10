# Частичный P24: clean build в одноразовой Arch VM

Целевая база: `eec7a56b6b03f2b022a87682f9097ce78b61fa0b` (main после #64).
Android исключён. Старый #57 проверял исходный e33-пакет и тестовую перепаковку;
его результат не является clean build текущей базы.

Рецепт обновлён на фиксированный main SHA, pkgrel увеличен до 2. SHA-256 архива
GitHub/codeload: `71e9c347ffc363da062b42b0c5ffc6ccb0a97b0988202ce21391dd1c2553a1d3`.
`CARGO_BUILD_JOBS=1` соответствует ограничениям стенда. Зависимости, команды установки и перечень payload не изменены.
Первый clean build выявил GNU linker archive-order failure CXX-Qt 0.10: четыре
`cxx_qt_init_*` symbols присутствовали в rlib, но forced initializer archives
ссылались на них позже в linker command. Рецепт использует `cargo rustc` для
desktop с четырьмя явными `--undefined`; helper/worker собираются прежним
`cargo build`. Производственные исходники не менялись. Failing log сохранён
в `build-red.log`; повторный makepkg использует ту же собственную VM и target. URL переключён на
официальный codeload после connect timeout GitHub archive endpoint; два
независимых скачивания codeload дали одинаковый SHA-256. Seal фиксирует новый
рецепт целиком; валидатор не ослаблен. Fixture синхронизирован с revision/pkgrel.
RED: прежний seal отверг изменённый рецепт (3 errors / 6 tests).
GREEN: после синхронизации seal/fixture 6 tests проходят, включая отказы на
изменённые metadata, небезопасные пути и shell mutations.

Runner `tools/arch_clean_vm_build.py` создаёт собственный task root с BSD flock,
скачивает официальный cloud image и проверяет SHA-256 плюс detached PGP signature
по primary Arch CI `1B9A16984A4E8CB448712D2AE0B78BF4326C6F8F` в отдельном GNUPGHOME.
VM: KVM/UEFI, 2 CPU, 4 GiB; отдельные overlay, seed, SSH key и loopback port.
Host-пакеты, сеть и пользовательский keyring не меняются. Все build dependencies
ставятся внутри VM; Rust toolchain 1.85.1 устанавливается rustup внутри VM.
Guest makepkg исполняется под host `with-rust-slot`; target отдельный, jobs=1,
flags generic (host native flags не наследуются). Исходный Cargo.lock обязателен.
Архив создаётся makepkg из source с закреплённым SHA/checksum, не перепаковкой ELF.

Evidence и финальный пакет: `local/validation/arch-clean/` в этом worktree,
вне TTL, не коммитятся. Ночной прогон 10–11 ноября завершён. Финальный пакет:
`yonik-0.1.0.r20261110-2-x86_64.pkg.tar.zst`, SHA-256
`e53cb758ca1b9a2167a80c90ca50b1ab5eb4f6b67c5c8ec01c2bf0ccb856e8b0`.
Это новый makepkg artifact; исторический test-only repack #57 с похожим pkgrel
не использовался. Его ELF и `.BUILDINFO` не переносились в VM.

| Проверка | Фактический результат |
| --- | --- |
| Official image SHA/PGP | PASS; image SHA `794eb4889b39e1f0ed7d5ae998b2a1ed2137db8cf8c3476cf2962c7437ecb003` |
| Clean source build | PASS; desktop 6м13с, helper/worker 2м56с; makepkg exit0 |
| Source provenance | PASS; source checksum повторно проверен в VM, embedded desktop ID `0.1.0+eec7a56b6b03f2b022a87682f9097ce78b61fa0b` |
| Recipe provenance | PASS; `.BUILDINFO pkgbuild_sha256sum` = `2cb9e296901b0f72baa230d1ec4c680e94d61ec4c453de136ddab68d6ea4be7a` |
| Generic x86-64 flags | Native flags не наследовались; ELF GNU property: `x86-64-baseline`, note linker `gold 1.16`; отдельная physical CPU matrix не выполнялась |
| Archive validator | PASS в VM и повторно на сохранённом финальном архиве на host |
| Установка/целостность | PASS; `pacman -U`, `pacman -Qkk`: 32 files, 0 altered |
| Units/autostart | PASS; `systemd-analyze verify`, helper inactive после установки |
| Qt из `/usr/bin` | PASS; непривилегированный builder, новый пустой HOME/XDG, offscreen/software; 142 steps, 43 events, 3 TCP EOF, 7 HWID requests |
| Rust fmt/clippy | PASS; Rust 1.85.1, fmt all, clippy release workspace/all-targets `-D warnings` (default features; GUI отдельно проверен build/smoke) |
| Пакетные regressions | PASS; 6 unittest, Ruff format/check, bash syntax |

Evidence: `build-red.log`, `build-green.log`, `binary-provenance.log` (включая
`.BUILDINFO`), `dependencies.log`, `rust-validation.log`, `install.log`,
`smoke.log`, `smoke/trace.jsonl` и screenshots, финальный пакет, SHA/PGP metadata.
`source-provenance.log` относится к первому рецепту до linker fix;
финальный hash рецепта зафиксирован в `binary-provenance.log` и `.BUILDINFO`.
После первой ошибки дальнейшие стадии выполнены отдельным `finalize.py` из
evidence; исправления стенда перенесены в постоянный runner. Исходный runner
возобновлён только для штатного выключения после своего RED status; его exit1
в `runner.log` не подменяется успехом. Статусы финальных стадий отдельно
зафиксированы в `final-status.json`.

Очистка: guest `cargo +1.85.1 clean` через host slot удалил 4701 файлов / 1.0 GiB.
До выключения проверено отсутствие cargo/rustc/cc1plus. Собственная VM exit0,
QEMU/runner завершены; свой gpg-agent остановлен, transient
`/var/tmp/codex-myrs/transient/yonik-arch-clean-eec7` удалён под root flock.
Ключи, seed, overlay и target не сохранены. Пакет и evidence находятся вне TTL.
Host installation, Docker, deploy, release upload, чужие VM/процессы не использовались.

Воспроизведение: `python tools/arch_clean_vm_build.py --task-root
/var/tmp/codex-myrs/transient/<новое-имя> --evidence <постоянный-каталог>
--arch-public-key <официальный-публичный-ключ-Arch-CI.asc>`.
Fingerprint сверяется с закреплённым primary независимо от имени файла.
Независимый review нового recipe seal остаётся за root перед merge.

P24 остаётся частичным: real helper registry/session acceptance принадлежит L08.
P25 не закрыт: #68 содержит 8 missing license / unknown composition; notices,
SPDX, release и побитовая воспроизводимость не приняты данным прогоном.
