# Владение TUN при native ATTACH

Зависимая база: PR #153, c7da3812aee35ae8d8fe730ae1f95caefdf3cf34.
Сохранён witness 1cf6aba обычным merge; история и чужие refs не переписаны.

Backend удерживает original Arc<BoundTun> до подтверждённого stop. Native Session
удерживает тот же owner; ATTACH формирует 32 байта metadata из проверенной очереди
и выдаёт один CLOEXEC duplicate. Caller metadata удалена из attach/serve.
Framing только Plain; Vnet10 отклоняется. Wire flags 0x1001 семантические:
IFF_PERSIST проверяется fixed ioctl, но не добавляется в frozen ABI 260.

Проверки:

- 15 session-тестов: handshake, malformed metadata, original-owner lifetime,
  CLOEXEC, timeout, cancel, EOF, revoke, replay.
- Изолированный user/net/mount/PID namespace: real bound queue проходит actual
  ATTACH/SCM_RIGHTS, cancel до/после attach и revoke возвращают FD count к baseline;
  original queue остаётся проверяемой. Проверяется отказ Vnet10 metadata.
- Backend namespace test: unknown stop удерживает custody, confirmed stop освобождает
  owner перед cleanup Journal.

Это acceptance механики владения, не production admission. Полная Session factory
остаётся закрытой до namespace/kernel/invocation admission. Producer RunAuthority
и start_authorized требуют отдельной интеграции W2 с доказанным owner context;
legacy by-name не запускается с занятой bound queue. Тестовые raw FD существуют
только под cfg(test). Полученный удалённым процессом SCM_RIGHTS FD невозможно
принудительно закрыть из helper: shutdown отзывает канал, cleanup ждёт stop worker.
Native positive persistent Plain campaign W1 остаётся отдельным pending leaf.
Cross-UID campaign не выполнялась.

Полные логи: /home/myrs/projects/yonik/local/orchestration/evidence/w4-native-tun-attach/.

Clippy all-targets с -D warnings не проходит на существующем
clippy::collapsible_match в yonik-engine/src/lib.rs:170. Scoped all-targets
--no-deps дополнительно обнаруживает существующий clippy::cloned_ref_to_slice_refs
в yonik-linux/tests/resource_fd_receiver.rs:147. Эти файлы не изменялись.

Scoped clippy --no-deps -p yonik-linux -p yonik-nsfs --lib -- -D warnings
прошёл; cargo fmt --all --check и git diff --check прошли.
Проверка с явным RUSTUP_TOOLCHAIN=1.97.0-x86_64-unknown-linux-gnu.
