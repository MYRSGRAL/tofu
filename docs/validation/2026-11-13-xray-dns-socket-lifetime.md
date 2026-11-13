# Xray: срок жизни внутреннего DNS socket

База: PR #156, `2c9dc795608081d6c8e30427c425b64a0e3fc5a3`.

Сохранённый full-builder log W4 (`second-xray-builder-logs/06-go.stdout.log`)
содержит отказы `TestNativeDNSConcurrentAuthority`, включая known/victim=0,
cache caller и cancel/victim=0. Отдельный запуск known на прежнем коде прошёл:
это зависимый от порядка исполнения дефект, а scheduler pause не доказан.

Доказанный путь: cacheExchange создаёт DNSExchangeContext с тем же закрытым
routingConsumer. doFetch получает ответ, публикует cache и закрывает done;
последний waiter вызывает cancel, также cancel вызывается при выходе exchange.
OwnSocket регистрировал callback отмены, вызывавший denyFlow всего consumer.
Если callback успевал до CheckAfterDNS или следующего cached lookup, даже Known
caller получал ErrDenied. Если отменялся ведущий caller, denyFlow закрывал также
общий DNS socket, хотя другой waiter продолжал ждать. TCP мог повторить exchange
(нарушение queries=1), UDP мог завершиться по существующему deadline без ответа.
Эти два пути наблюдались при промежуточном исправлении callback без изоляции
списка handles; evidence green.log содержит TCP queries=2 и UDP deadline.

Исправление в OwnSocket: внутренний DNS socket остаётся под Session.Track и
session revoke, но не входит в список caller handles. Его context callback
освобождает только собственный handle. Отмена обычного caller по-прежнему
фиксирует denyFlow; Unknown/Unavailable не превращаются в Known. Общий wire
REVOKE по-прежнему терминален для всех session handles. ABI, pins, caps, TTL,
deadlines, retries и assertions не изменены.

Новый TestDNSLastWaiterKeepsCallerAuthority детерминированно получает EOF на
внутреннем socket после cancel, затем проверяет CheckAfterDNS, BeforeSocketDial,
живой соседний socket и терминальность Session.Revoke. До исправления: RED на
cache authority после last waiter. После: GREEN вместе со всей матрицей из 72
сценариев. Scoped race включает admitted session cancel/Unknown/Unavailable и
wire REVOKE, native/authenticated DNS и opened-flow revoke.

Evidence вне TTL: `/home/myrs/.local/share/codex-artifacts/xray-dns-isolation-*`.
Это компонентные проверки native API; production issuer, foreign UID и физический
TUN acceptance не устанавливались. Rust W2/W4 не изменялся; persistent plain
campaign и baseline повторно не запускались.

Полный builder запущен один раз: остановлен его штатным лимитом 180 секунд
во время холодной компиляции зависимостей (07-go, stdout пуст, stderr downloads).
GREEN builder не заявляется; неизменённый builder повторно не запускался.
18 builder unit tests, Ruff check/format и gofmt прошли.
Полный suite из подготовленного source с обычным прогретым Go cache:
`go test -p=1 -mod=readonly ./common/yonikbroker ./app/dispatcher ./proxy/tun`
прошёл. Это package suite, а не успешное завершение full builder wrapper.
