# P09: независимый Xray host-SIGKILL baseline

Результат: **survives**, 14 ноября 2026. Один Xray baseline/control → crash run,
без Rust build, unsafe patch или повторного sing-box запуска. Не full P09/native
PASS и не проверка предложенного исправления.

## Предварительная read-only сверка

Retained normal DesktopHost ELF из sing-box witness:
`/home/myrs/projects/yonik/local/evidence/p09-host-sigkill-8ea826a/local_socks_alternating`.
SHA256 `27cc3f3169f79af4cb636387a08ff26d4f9fbb5e984a36028fca2f0b5b642cfe`
совпал с исходным binding. Все frozen source/Cargo hashes из его manifest
совпали с текущими файлами. Production base:
`8ea826a04a745c8916e14a4329588c75f66b683e`.

Существующий Rust example `local_socks_alternating` принимает engine из argv,
использует обычный DesktopHost и command `set_engine`. Backend уже содержит
Xray path/pin. Локальный `xray-26.10.31/xray` имеет ожидаемый SHA256
`cc4b84d3069569f297a61f04b914f2bb37e9d202fd7b84cb50def1fc30fd40e1`.
Следовательно, API/ELF gap отсутствует. Изменён только собственный Python harness:
`--engine xray` выбирает существующий core/peer; default sing-box сохранён.
Observation offsets заданы от общего monotonic start, последняя проверка ≤3,2s.

## Actual результаты до teardown изоляции

- Positive: normal host PID19/core46 внутри namespace, Connected → explicit Stop
  → pidfd ready, core `/proc` отсутствует, children host пусты, rebind успешен;
  shutdown/closed, host exit0.
- Negative canary перед каждым Stop/crash: core жив с той же identity, pidfd не
  ready, LISTEN inode принадлежит его FD; bind с SO_REUSEADDR отвергнут. Это actual
  live child/occupied port, не synthetic state assertion.
- Crash: normal host PID58 получил actual SIGKILL, wait=-9. Core PID85,
  starttime `6604749`, тот же pinned ELF и собственный SOCKS listener остались
  живы на 0,016 / 1,016 / 2,016 / 3,016 секунды. Port38895 занят. **survives**.
- Обязательный cleanup: supervisor подтвердил identity, отправил SIGKILL через
  held pidfd, waitpid вернул PID85/status9; final rebind успешен,
  `final_children_empty=true` до выхода namespace. Orphan не оставлен.

Supervisor — PID1 собственной PID/mount/user namespace с текущим UID — пережил
host. Private D-Bus/XDG, HTTPS/CA и pinned VLESS peer были живы всё время crash
наблюдения. Только собственные loopback sockets, без изменения host network,
keyring/services/TUN/worker/native/N5. Namespace exit не использован как продуктовый
cleanup oracle. Runtime exit0, stderr/log не содержат ошибки.

## Binding и воспроизведение

Отдельный evidence вне TTL:
`/home/myrs/projects/yonik/local/evidence/p09-host-sigkill-xray-8ea826a/`.
`binding.json` фиксирует retained ELF SHA, exact production base, pre-execution
Python/imported recipe hashes и hash исходного sing binding. `result.json`
содержит snapshots, core identity, controls, observations и cleanup;
`runtime.log`, `stop.stderr`, `sigkill.stderr`, `post-run.json` сохранены рядом.
Ни sing evidence, ни proposal b7b9, ни старые reports не перезаписаны.

```sh
python crates/yonik-desktop/tests/p09_host_sigkill.py \
  /home/myrs/projects/yonik/local/evidence/p09-host-sigkill-8ea826a/local_socks_alternating \
  /home/myrs/projects/yonik/local/engines \
  /home/myrs/projects/yonik/local/evidence/p09-host-sigkill-xray-8ea826a-replay \
  --engine xray
```

Проверки: actual Xray matrix exit0, Ruff check/format, git diff --check;
source/binary hashes повторно сверены после runtime. Нет нового build/transient
каталога для очистки; временные fixtures, bus/peer и все дети supervisor закрыты.
Frozen sing finding `59ece58809b749804f1e4fc41eb83bb8be55b6e1` и proposal
`c33d036ef9756508b1381d5e8e5a39fa76a5de1d` остаются отдельными refs.
Unsafe patch SHA256
`b7b9c2d0f7bb08db43864fb9f206ae75ba33df5e1662316cb921b7020b13c8a2`
не применён и этим witness не разрешается. Validate/Apply barriers и полная
native host-loss часть остаются OPEN.
