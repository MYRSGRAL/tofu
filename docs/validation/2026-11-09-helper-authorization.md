# D-Bus/polkit граница авторизации helper

Компонент `tofu_linux::helper` выполняет настоящие запросы D-Bus/polkit, но не
устанавливает системный сервис и не применяет сеть. Контракт и границы описаны в
[helper-authorization](../specs/helper-authorization.md).

## Реализовано

`PeerAuthorizer::system(Cancellation)` использует канонический system socket.
`authorize(&Message, NetworkAction, Prompt, Cancellation)` получает UID у bus daemon,
закрепляет root-owned authority, проверяет raw reply senders до десериализации,
запрашивает только фиксированное действие и после ответа сверяет peer/authority.
Возвращаемый PeerIdentity — идентичность на момент запроса, не reusable permission.

Число активных проверок ограничено четырьмя; ожидание слота тоже отменяемо и
ограничено deadline. Неинтерактивный deadline 5 с, явно интерактивный 120 с,
setup-операции дополнительно ограничены 5 с. Входная body ограничена 64 KiB,
ответы — 4096 байт после получения frame библиотекой zbus. Проверка большого
frame до его транспортной аллокации этим компонентом не доказана.

Каждая remote check имеет собственный случайный cancellation ID. При отмене
или timeout CancelCheckAuthorization адресуется исходному unique owner и ждёт
не более 250 мс. Отсутствие подтверждённой remote cancel не разрешает локальный
эффект. Поддерживается закрытие источника Cancellation; поздний allow игнорируется.

## Проверки

Rust 1.85.1: RED E0432 до реализации API; затем 13 GREEN private-bus тестов.
Свой dbus-daemon и fake authority не используют system bus/polkit пользователя.
Проверены:

- wire subject с фактическим unique sender, UID из daemon вместо UID999999 в
  вымышленном payload, allow/deny/challenge, фиксированные action/flags и пустые details;
- отсутствие sender, неверный тип сообщения, oversized body и недоверенный authority;
- malformed/oversized ответ, исчезновение peer и замена authority во время проверки;
- timeout, отмена и закрытие cancellation owner, поздний положительный ответ;
- отмена ожидающей слота проверки до remote call;
- независимые случайные cancellation IDs параллельных проверок;
- отмена исходного owner после передачи well-known имени другому peer;
- настоящий forged allow от другого peer на private bus: ответ отвергается по
  sender, пока правильный authority отвечает deny.

Исходники закреплённого zbus4.4.0 показали сопоставление PendingMethodCall по
reply_serial без проверки sender. Поэтому проверка sender выполняется явно
в компоненте до разбора решения; корректность private-bus rejection проверена
реальным обменом, а не только подставным Result.

Root workspace tests, workspace clippy all-targets с `-D warnings`, fmt и
проверка diff прошли на 1.85.1. Перед fast-forward к merged main подтверждено,
что tree базового a0dabd8 совпадает с tree merge146a725; рабочий source не менялся.
Локальные логи: `local/validation/helper-authorization/`.

## Что дальше

Компонент принимает Message от доверенного будущего server dispatcher; вызов с
самостоятельно построенным внутри процесса header не является аутентификацией
сетевого transport. Public API не экспортирует test constructor, authority UID
или адрес частной шины.

PeerIdentity не отменяет необходимость свежей авторизации перед Prepare/Apply,
проверки immutable artifact, generation и ревокации. Установка polkit action files,
D-Bus service policy, обработчик команд, trusted engine registry, реальные сетевые
эффекты и возврат исходной сети остаются следующими частями полного helper.
