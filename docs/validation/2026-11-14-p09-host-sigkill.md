# P09: actual host SIGKILL — core survives

14 ноября 2026. Production/source freeze `8ea826a`; отдельный docs-only head
`5560ab7` не меняет build inputs. Это concrete LocalSocks finding, не полный
P09/native host-loss verdict. Validate/Apply barriers остаются OPEN.

## Матрица и результат

Один core: sing-box 1.14.4, SHA256
`1264361b5f8868c8768f7555d3e77e20118bf503c41ac08bb290a0d99056aa3a`.
Обычный `DesktopHost::start` существующего example `local_socks_alternating`;
не test-support/hook/production patch. Synthetic subscription, ephemeral key,
private D-Bus/XDG, собственные VLESS peer и HTTPS/CA, loopback ephemeral ports.
Нет host keyring, TUN, system service или изменения host settings/network.

| Случай | Наблюдение до выхода namespace |
| --- | --- |
| Live negative canary, оба Connect | pidfd не ready, PID/starttime/exe совпадают, LISTEN inode принадлежит core, bind того же порта отвергнут |
| Positive explicit Stop | Connected → Stop ACK; pidfd ready; `/proc/PID` отсутствует, host children пусты; rebind успешен; shutdown/closed и host exit 0 |
| Actual host SIGKILL | `Popen.kill`, host wait = -9. Core остаётся живым с прежней identity и собственным listener на 0,036 / 1,071 / 2,107 / 3,143 секунды; порт занят. Verdict **survives** |
| Обязательная уборка finding | Supervisor проверил identity, послал SIGKILL через удерживаемый pidfd core, waitpid вернул PID69/status9; rebind успешен, все дети PID1 отсутствуют |

PID указаны внутри собственной PID namespace: normal host19/core37,
crash host51/core69. После host death supervisor продолжал быть PID1;
private bus/peer/HTTPS и namespace оставались живы во время наблюдения.
Поэтому namespace auto-teardown не является причиной core cleanup или verdict.
Mount/PID/user namespaces сохраняют текущий UID; network namespace не создаётся,
системная сеть не меняется, стенд использует только свои loopback sockets.
Новых capabilities или syscalls не добавлено.

## Source/build/evidence binding

Один focused build Rust 1.85.1:
`cargo +1.85.1 build --locked -p yonik-desktop --example local_socks_alternating -j1`.
Shared `with-rust-slot` (два глобальных слота), jobs1, пустые RUSTC_WRAPPER и
RUSTC_WORKSPACE_WRAPPER, BSD flock корня собственного transient task.
Manifest всех tracked crates/Cargo inputs записан до сборки; после сборки
каждый hash совпал, code edits во время сборки отсутствовали.
Retained host ELF SHA256:
`27cc3f3169f79af4cb636387a08ff26d4f9fbb5e984a36028fca2f0b5b642cfe`.
Новый Python harness и imported recipes захешированы перед runtime отдельно.

Evidence вне TTL:
`/home/myrs/projects/yonik/local/evidence/p09-host-sigkill-8ea826a/`:
`freeze.json`, `binding.json`, retained `local_socks_alternating`, `build.log`,
`runtime-source-binding.json`, `runtime.log`, `result.json`, case stderr.
Raw snapshots synthetic, не содержат пользовательских credentials.
Команда воспроизведения:

```sh
python crates/yonik-desktop/tests/p09_host_sigkill.py \
  /home/myrs/projects/yonik/local/evidence/p09-host-sigkill-8ea826a/local_socks_alternating \
  /home/myrs/projects/yonik/local/engines \
  /home/myrs/projects/yonik/local/evidence/p09-host-sigkill-8ea826a-replay
```

Первый запуск остановился на normal Stop: bind без SO_REUSEADDR столкнулся с
TIME_WAIT после успешного exit/reap. Crash-case тогда не запускался. Evidence
сохранено в `attempt1-timewait/`. Oracle исправлен на принятый #173 rebind с
SO_REUSEADDR; повтор actual матрицы exit0. Живой listener и с этим флагом
отвергает bind — обе negative canary это подтвердили. Новая Rust-сборка не нужна.

## Минимальный fix-contract для root, без реализации

Обычный await Stop подтверждает cleanup; гибель процесса host не исполняет
Rust Drop и обрывает его runtime. Finding требует владельца core, который
переживает host, обнаруживает loss control channel и выполняет terminate/wait
для своего процесса до cleanup ACK. При проектировании сначала рассмотреть
отдельный supervisor с существующими safe Rust process/channel API, с закрытием
канала владельца, bounded TERM→KILL→wait и identity-bound ownership. Повторный
Stop/loss должен быть идемпотентным; никакого ACK до подтверждённого exit/reap.
Конкретный API и место интеграции должен согласовать root с владельцами shared
файлов; это не разрешение добавлять supervisor в production сейчас.

PDEATHSIG, новый syscall, systemd user unit, hooks/backend/session правки здесь
не применялись. Один sing-box case не доказывает поведение Xray, crash на других
стадиях, descendants, native worker/cgroup или cleanup после supervisor crash.
Нормативная system/native часть остаётся OPEN. 100 циклов и P03 не повторялись.

Проверки финального harness: Ruff check/format и git diff --check — PASS.
После runtime импортированные исходники повторно сверены с pre-execution hashes.
Перед cleanup активные exe/cwd/fd ссылки на build root отсутствовали;
штатный cargo clean выполнен под flock/shared slot, transient root удалён.
Retained ELF, логи и evidence остаются вне TTL. `pre-clean.json`, `clean.log`.
