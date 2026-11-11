# Внутренняя семантика proof guard

Компонент переносится патчами в Xray/sing-box и декодирует frozen wire v1.
Он пока не подключён к runtime hooks и не открывает FD,
не подтверждает featureack и не заменяет Rust broker. Evidence конструирует только
будущий trusted decoder; публичный Go struct сам по себе не security authority.
Новых зависимостей нет: тестируется стандартной библиотекой с GO111MODULE=off.

Guard проверяется перед routing, после DNS и непосредственно перед новым dial.
Flow содержит исходный tuple до DNS/sniff переписывания. IPv4-mapped адреса
должны быть канонизированы trusted вызывающим кодом; смешанная семья, zone,
нулевые ports, неизвестный protocol отвергаются. Non-app control не требует proof.

Session.Context передаётся DNS и dial. Track вызывается до публикации TCP/UDP
flow и повторно проверяет proof; при отказе вызывающий код закрывает flow.
При успехе Session владеет закрытием через Handle/Release. EOF, cancel, epoch
revoke и broker death должны вызывать Revoke: он отменяет pending I/O и закрывает
все зарегистрированные flows, включая UDP. Close вызывается вне mutex,
concurrent Release/Revoke используют sync.Once. Лимит fixture — 4096 flows;
production budget ещё требует согласования с Root.

Это component evidence. Реальные upstream hook call sites, DNS/dial propagation,
FD custody и полная helper VM матрица пока не доказаны. Будущий transport должен
проверять binding, kernel proof и его монотонные временные границы до Evidence.
