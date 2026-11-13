# N4: ограниченное предложение test-only consumer bridge

Статус: NEEDS ROOT CONTRACT; runtime consumer acceptance НЕ выполнена.
Зависимая база PR171: `ca801bc1b6916522954b6e97a081b305f7b0bfe7`.
В отдельную ветку обычным merge включён main `ec363a94799eab491fce76dae0ae85a30438802b`
(PR126 с Go158). PR171 и исходные refs не изменялись. PR167
`ce356b452675036fab3388ef64ea03f8f00910ce` остаётся accepted build evidence,
не доказательством нового связанного consumer path.

## Конкретный разрыв

Rust `managed_network/tun_custody_tests.rs::real_journal_queue_custody_controls`
создаёт Journal-owned queue, передаёт Arc<BoundTun> в
`native_session/transport.rs::bound_terminal_controls`. Настоящий serve отправляет
ATTACH; handshake проверяет единственный SCM_RIGHTS FD и CLOEXEC, затем закрывает
полученный FD до возврата. Эти kernel controls уже приняты, повтор не нужен.

Оба Go patches содержат private `closedTUNFixtureMode`: fixture сама создаёт
`ynkfixture0`, issuer socketpair, bootstrap record и вызывает private
`closedAttach`, получая private routingConsumer. Actual constructors используют
этот context. `PersistentPlainTUNFixture` не принимает внешний endpoint.
В Xray это `common/yonikbroker/export_test.go` и `owned_tun_linux_test.go`
внутри native-broker-codec.patch; те же symbols есть в sing-box patch.
Простая подстановка Rust-received FD не проверит actual authenticated ATTACH и
потребует самостоятельно изготовить приватный context. Готовый production ELF
PR167 не предоставляет test issuer: выдавать его за доступный fixture launcher нельзя.

## Предлагаемый контракт, требует решения root до реализации

1. Разрешить только test overlay обоих pinned Go packages (не production patch,
   не source-lock/package hashes): заменить внутреннее создание fixture на
   private test bridge к отдельному Rust fixture process. Нужен test binary
   каждого actual pipeline package; production ELF167 сохранить отдельно.
2. Rust cfg(test) fixture создаёт isolated Journal, Plain BoundTun и Session
   через существующую private fixture с заменой TunRole на Bound. Отдельный
   fixture process владеет sender endpoint и original Arc до stop consumer.
   Никаких новых public FD APIs или production factory.
3. Явно согласовать fixture-only bootstrap: existing 192-byte record с binding
   из Rust Session, live issuer PID/starttime/pidfd, один SEQPACKET endpoint.
   Go test bridge вызывает существующий private closedAttach и сохраняет
   проверки SCM_CREDENTIALS, PID/starttime, binding, single CLOEXEC FD, ACK.
   Оба процесса находятся в одном UID mapping внутри своей user/net/mount ns;
   namespace-local UID0 удовлетворяет текущему credential validator.
4. Использовать Journal interface name/MTU вместо ynkfixture0; не передавать
   caller TUN FD. Actual TCP/UDP pipeline получает context только через
   closedAttach. Начальный scope non-app routing, без фиктивного Known broker
   proof: это consumer witness, не lookup/admission matrix.
5. Positive обоих cores: TCP/UDP echo и blocked writer; wrong ATTACH metadata
   отклоняется до constructor; cancel/revoke прекращают traffic и writer.
   Consumer подтверждает stop/exit, затем Session освобождает owner, original
   Arc drop предшествует Journal cleanup. Проверять FD baseline обоих процессов,
   отсутствие TUN reader/copy goroutines, сохранность original до stop.

Не открывать full production Session/bootstrap admission; не добавлять unsafe,
capabilities, новые ioctl wrappers, production mint или shared wire изменения.
Новые 16 probes, cross-UID campaign и Vnet claim запрещены. Если bridge потребует
изменения closedAttach или routingConsumer production semantics — новый proposal.

## NEXT и критерий возобновления

Root должен утвердить именно Go test overlays и fixture-only bootstrap bridge
из пунктов 1–4; после этого реализовать/проверить пункт 5. Разрешённые ранее
cfg(test) hooks в native_session/worker сами по себе не создают Go context.
Это предложение не утверждает техническую невозможность test bridge; оно
фиксирует требуемую дополнительную область и authority contract.

При исполнении: источник Go158 и manifests167 сверить до сборки, сохранить
хеши overlays, точные команды, source manifest, test ELF SHA256 и raw logs вне
TTL. Rust jobs=1 через existing two-slot wrapper, внешний transient target под
flock корня; cargo clean после сохранения evidence. В этой попытке сборки и
runtime не запускались, transient-каталог не создавался.
