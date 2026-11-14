# P07/P20: bounded actual GUI semantic persistence с normal Host/private SS

Persistence-only leaf **PASS** на production sources accepted
`3247e6c32a5ed83ec5d812dfc1f70695d3f2b3b4` плюс новый test example.
Полные P07/P20/A-04/A-07 не повышаются. Port structural gap из PR207 сохраняется;
[минимальный port/protocol contract](2026-11-14-p07-port-protocol-contract-proposal.md)
подготовлен для root review, без domain/wire/schema изменений.

## Producer и oracle

`examples/p07_persistence.rs` включает исходные production bridge/dispatch_error;
HostConfig обычный, без ephemeral key, custom repository/provider и actor writes.
`run_application(..., true, ...)` включает существующий UI smoke driver и trace;
это не вызов production main с `--smoke-test`, который выбирает ephemeral key.
Initial/fresh Ready gates требуют `ephemeral=false`. Credential path вызывает
normal `subscription_master_key`, затем encrypted repository.

Собственный private KSecretD, bus без service activation directories, отдельные
HOME/XDG, user/net namespace (только loopback без probes), offscreen/software Qt.
KSecretD запускается P06 --pam-login recipe, random bootstrap идёт через pipe.
Inspector проверяет bus owner PID/UID, unlocked collection, item path/attributes;
не читает GetSecret, ключ, ciphertext или memory. До daemon имя отсутствует,
после terminate/wait снова отсутствует. Внутри userns UID=0, host UID не менялся.

Production driver принимает commands только через обычный bridge dispatch.
Синтетический seed двух правил и maintenance backup выполнены этим путём;
порядок, переключения режимов, выбор/подтверждение правила, enabled checkbox и
quit выполняются настоящими Qt keyboard Space/Tab events через существующий
smoke::perform, focus/reveal и production handlers. Это не физический Wayland
input/latency proof. Текст/Actor state/properties не инъецируются harness.

Full equality проверяется **в actual snapshot gates production driver**, а не
сравнением двух caller-констант. `when.routing` требует точного QJsonObject equality:
ID, порядок rules/внутренних массивов, enabled, actions, default_action, conditions,
platform_sources и ru_direct=null. Следующий UI step допускается лишь после
равенства. ACK gates требуют request_id/saved_id/error; поля и видимость проверяет
`ui_when`. Trace хранит только штатные краткие metadata + step/closed, **не полный
routing snapshot**. Доказательство — exact scripts + успешные gates/terminal step,
связанные с frozen source driver; полный raw snapshot log не заявляется.

## Единственная финальная короткая серия: три fresh процесса

1. Normal Ready создаёт DB/key item. Maintenance создаёт verified backup3.
   Production edit_routing seed #41: disabled, Block, два executable,
   exact/suffix domain, IPv4/IPv6 CIDR и AndroidPackage selector. #42: enabled,
   Direct, domain-only, пустой selector array. Default Proxy.
2. Production UI `routingUp42` переставляет [41,42] → [42,41], routing rev 2→3.
   Edit41 → Simple→Advanced→Simple. В Advanced видны три группы, в Simple только
   executable; содержимое всех трёх групп одинаково. На переходах полная policy
   и rev3 неизменны. Production Save даёт ACK request_id=2, saved_id=41, error=null
   и rev4; full equality подтверждает disabled selector/order/conditions. Quit →
   Closed, успешный exit; процесс завершён до следующего запуска.
3. Fresh процесс на том же private SS/DB требует exact disabled draft/rev4 при
   Ready. UI включает #41 и Save: ACK request_id=1, saved_id=41, error=null, rev5.
   Сохранение enabled unsupported draft нормативно разрешено. Production bridge
   Connect на **существующей activation boundary** даёт unsupported_option,
   can_stop=false; exact draft/rev5 сохраняются. Этот Connect передан production
   command через driver; физическая кнопка Connect и native Apply не заявляются.
4. Третий fresh процесс требует exact enabled draft/rev5 при Ready, снова
   проверяет actual fields/Advanced/Simple и завершает normal Quit без записи.

Во всех трёх post-exit inspection: один и тот же owner PID, item
`/org/freedesktop/secrets/collection/kdewallet/0`, count=1, unlocked и одинаковые
application/purpose/schema attributes. Fresh-process расшифровка full draft
доказывает доступность нормального ключа, inspector его не получает. Все owned
GUI завершены до завершения daemon/bus; cleanup result подтверждён.

Active native generation здесь не создаётся. Negative доказывает ранний
activation refusal без потери draft; он не доказывает rollback существующего
native runtime. Ports, TUN/N5, process/domain enforcement, full A04/A07, прежние
потоки, physical UI/GPU/energy остаются вне scope. Offline Save не native Apply.

## История ограниченных fixture/oracle корректировок

- actual-1: KSecretD создал общий data с mode0755; Host отказал до Ready. Fixture
  заранее создаёт собственный data0700. Production проверки не ослаблялись.
- actual-2: driver завершился exit2 на mouse-click UI reorder после model update;
  layout/geometry timing — предполагаемая причина, точный Qt diagnostic не был
  доступен. Заменены действия на штатные keyboard Space events, не прямые сигналы.
- actual-3: три процесса PASS для contents/gates; видимость групп не проверялась.
- actual-4: добавлены явные visible gates Simple/Advanced и ephemeral=false; PASS.
- final: тот же ELF, окончательный typed runner и owned-process cleanup flag;
  три процесса PASS. Только final используется для текущего witness.

Отказы и промежуточные данные сохранены вне TTL; они не переатрибутируются final
runner. Никакая P03 3×100 или полная A08 campaign не повторялась.

## Binding, проверки и воспроизведение

[Bindings](p07-persistence/bindings.json), [compiled source hashes](p07-persistence/compiled-sources.json),
[result](p07-persistence/result.json), exact scripts и traces находятся рядом.
Новый единственный GUI ELF собран Rust1.85.1, offline/locked, jobs1 через
with-rust-slot, CARGO_TARGET_DIR в own transient root под BSD flock корня.
394 compiled closure файла зафиксированы до build, побайтово неизменны после
build и runtime. Production files/Cargo.lock не менялись; harness не входит
в Rust compiled closure и дорабатывался отдельно от неё.

Воспроизведение с retained ELF:

```sh
unshare --user --map-root-user --net python3 tools/p07_persistence.py \
  ABS_ELF ABS_NEW_OUTPUT
```

Exact source archive, ELF, pre/post manifests, logs и failed attempts сохранены
в `local/validation/p07-persistence/` worktree e330 вне TTL. Это новый binding
к accepted production source, не reuse старого A08/P03 ELF с новой атрибуцией.

Проверки: focused Rust1.85.1 Clippy `--example p07_persistence --features gui
-- -D warnings` PASS (не all-targets); rustfmt --check нового example PASS;
Ruff0.15.20 check/format --check нового runner PASS; final integration witness PASS;
`git diff --check` PASS. Qt/GCC16 C++ warnings сохранены в build/clippy logs;
Rust Clippy warnings отсутствуют. Отдельные unit tests неизменной production
логики не повторялись; новый harness проверен actual GUI запуском.

Перед очисткой `/proc` scan не нашёл exe/cwd/fd ссылок на own transient root;
retained ELF не ссылается на него через runtime libraries. После завершения build,
Clippy и всех own GUI/SS процессов `cargo +1.85.1 clean` под slot/flock удалил
7585 файлов / 3.1GiB. Пустой own transient root удалён. ELF/source/raw evidence
сохранились вне TTL; чужие исходники, артефакты и процессы не очищались.
