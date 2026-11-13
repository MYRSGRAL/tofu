# FR07: короткий integrated Linux witness

База: `bf97e32e639ab3b01ca2feb119c7d103441af3f4` (main с #147/#148/#151).
Отдельная ветка `test/fr07-integrated-witness`; `task/helper-tun-custody`
на `c7da3812aee35ae8d8fe730ae1f95caefdf3cf34` сохранена без изменений.

`manual_encrypted_connect.py --short` ограничивает существующий harness одним
VMess WS/TLS профилем и Xray 26.10.31. Сохраняются actual Qt manual fields,
review, verified backup, approve, encrypted repository, завершение GUI,
новый процесс, select и UI Connect. Проверяются actual check/run writes и hash
исполняемого memfd pinned core. Peer sing-box 1.14.4 принимает правильные
credentials и отклоняет неправильные. В обоих случаях direct TCP echo отдельно
доступен; поэтому bypass не может пройти отрицательный контроль.

Stop — click `serversConnectButton` при `can_stop=true`. Только после
Disconnected/can_stop=false отправляется shutdown. Проверяются terminal closed,
исчезновение core PID и повторный bind SOCKS listener. Fixture затем вызывает
штатный host shutdown/wait_closed; backend Stop вместо UI Stop не используется.

Это свежая сборка существующего `tray_cleanup_negative` с `gui,test-support`
от указанной базы, а не retained installed `a8c5a29`. Production DesktopHost,
Bridge implementation fixture и production QML используются в существующей
композиции harness; синтетический ключ `[43;32]` нужен для двух процессов.
Это **не** доказательство Secret Service custody: оно принято отдельно в #147.

Дополнительный `local_lifecycle_smoke.py` проверяет rename/select/reopen/delete
локального источника и disabled refresh/HWID controls. Точный существующий
backend-тест `local_text_partial_approval_reopen_and_network_controls_fail_closed`
проверяет отказ schedule/refresh/source_change/HWID, неизменность revision и
счётчика HTTP, с положительным remote refresh control. Широкие матрицы
#135/#139/#140/#137 не повторяются.

Evidence вне TTL: `/home/myrs/projects/yonik/local/evidence/fr07-integrated`.
Сборка: `CARGO_BUILD_JOBS=1`, внешний target под flock корня собственного
transient-каталога, через общий `with-rust-slot`. Source binding и hashes
сохранены рядом с build/run logs и UI traces.

Воспроизведение traffic (под flock task root):

```sh
unshare --user --map-root-user --net dbus-run-session -- \
  python crates/yonik-desktop/tests/manual_encrypted_connect.py \
  "$evidence/tray_cleanup_negative" "$engines" "$task_root" "$evidence" --short
```

Граница результата: integrated FR07 manual→encrypted store→новый GUI→traffic→
UI Stop плюс local-source lifecycle/remote-operation denial. UDP и cross-product
протоколов не являются обязательными по requirements audit a38; этот witness
не заявляет полную Linux acceptance, Android/physical, P07/preset reopen,
production packaging или новую проверку Secret Service.

## Проверки и ограничения инструментария

Traffic: `xray-ws-valid` и `xray-ws-wrong-peer` прошли; оба terminal closed,
core reaped, listener освобождён. Lifecycle: оба процесса завершились closed,
после удаления profiles/sources равны нулю. Компактный результат:
[`fr07-integrated.json`](../evidence/fr07-integrated.json).

Ruff 0.17.4 check/format --check и `git diff --check` прошли. Свежая GUI fixture
собрана успешно; системный GCC 16 выдаёт предупреждения Qt/CXX (`-Wsfinae-incomplete`).
Попытка собрать backend integration test с `gui,test-support` не слинковалась
из-за undefined CXX/Qt bridge symbols. Это не исправлялось в production;
backend denial выполняется отдельно без GUI feature:

```sh
cargo test --locked -p yonik-desktop --features test-support --test backend \
  local_text_partial_approval_reopen_and_network_controls_fail_closed -- --exact
```

Backend denial: `1 passed; 0 failed; 55 filtered out`, свежий headless build.
Перед очисткой `/proc` refs на собственный transient root отсутствовали
(`cleanup-precheck.json`); binary и evidence сохранены вне TTL. Выполнен
`cargo clean` без global slot, собственный transient root удалён.
