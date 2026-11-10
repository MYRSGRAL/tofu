# Устойчивая политика и восстановление helper

Продолжение P12: policy payload хранится вместе с ресурсами в одном атомарном
журнале. Внешние Apply/Unlock по-прежнему должны авторизоваться системным helper;
свежие topology/engine bindings не заменяются содержимым файла.

## Schema v2

Один private journal содержит Ownership, bounded resource records,
монотонный policy_sequence и не более одного PublicPolicyRecord. Последний
ссылается на существующую собственную inet table и хранит revision и фазу:

- Present: желаемая политика и, при замене, предыдущая.
- Stable: политика, чьи фактические правила были подтверждены.
- Removing: до двух возможных политик после прерванной замены, которые разрешено
  распознать при явном Unlock.

Политика сериализуется типизированно, без команд и сырого nft JSON. Декодирование
проходит через те же валидирующие конструкторы; ограничения CIDR, индексов,
портов, mark, количества исключений и общего файла сохраняются. Неизвестные
поля и недопустимые связи phase/resource/state отклоняются. Debug состояния
не раскрывает payload. Права остаются 0700/0600, с FD-relative доступом,
exclusive writer lock, file/parent fsync и atomic rename.

Schema v1 явно мигрирует в v2 без выдумывания отсутствующего payload. Старый
public resource без политики остаётся неоднозначным: recovery не возвращает
NoPolicy, а новый Apply не присваивает ему произвольную политику. Это журнал
сетевого helper; schema пользовательской SQLite и домены Secret Service не меняются.

## Переходы и эффекты

`begin_public_policy` атомарно сохраняет Present и resource intent до эффекта.
Повтор того же pending desired использует прежнюю revision. Другой desired
не вытесняет неопределённый результат. Новый запрос из Stable создаёт следующую
revision; переполнение счётчика — отказ. Cleanup не превращается обратно в Apply.

`apply_durable_public_policy` распознаёт старое/новое содержимое, создаёт или
атомарно заменяет собственную таблицу и подтверждает Stable только после полной
сверки правил. Низкоуровневые block-операции закрыты внутри nft-модуля, чтобы
будущий service использовал entrypoint с durable payload.

`begin_public_unlock` сохраняет Removing и cleanup intent до удаления.
`unlock_durable_public_policy` пробует только сохранённые кандидаты и их полную
метку/правила; чужая таблица не удаляется по одному имени. Подтверждённое отсутствие
атомарно удаляет policy slot и resource record, сохраняя policy_sequence.
Повтор без managed public records — NoPolicy; legacy/untracked records не
объявляются успешно очищенными. Generic cleanup/Absent не обходят Removing.

## Startup recovery

`recover_public_policy` не создаёт, не заменяет и не удаляет сетевые объекты.
Он возвращает DesiredRules, PreviousRules, Missing или UnlockPending; при
отсутствии учтённой политики — NoPolicy, при неоднозначности — явную ошибку.
Совпавший desired может завершить историческую запись Stable. Previous не
подтверждает применение нового desired. Missing при Removing сохраняет намерение
до явного Unlock. Отмена или ошибка новой сверки сбрасывает старое ephemeral
подтверждение до первого await.
Poison после неопределённой durable-записи блокирует также быстрые ответы
NoPolicy/NoOp Unlock: прежний пустой snapshot не считается достоверным.

DesiredRules подтверждает именно kernel rules, а не актуальность VPN/TUN,
маршрутов или переиспользованного ifindex. Поэтому startup не воспроизводит
сохранённые исключения вслепую. После независимой проверки окружения доверенный
helper явно продолжает Apply либо завершает авторизованный Unlock.

## Отмена и область crash-гарантий

В одном живом runtime все nft executor используют общий permit, удерживаемый
owner-task до kill/wait Child. Новый экземпляр не обгоняет отменённую команду.
Это сериализация control plane, не пакетного datapath.

Межпроцессное fencing после SIGKILL во время живого nft subprocess ещё требует
systemd/cgroup lifecycle, остановки старой группы и подтверждения reap до старта
нового helper. Проверки шести границ между завершёнными операциями не доказывают
этот случай. Полная P12-приёмка остаётся открытой до service/IPC/host integration.
