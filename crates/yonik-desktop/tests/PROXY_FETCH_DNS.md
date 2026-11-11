# DNS обновлений Proxy

В «Настройки → Подключение» можно сохранить один literal `IP:порт` или
`[IPv6]:порт` DNS-over-TCP upstream. Менять его можно после отключения.
Пустое поле сохраняет `None`: доменная подписка Proxy отклоняется до lookup.
Адреса проверяются существующим `FetchHost::with_proxy_dns`; порт 0 запрещён.
Старые preferences версии 1 без поля `proxy_fetch_dns` читаются как `None`.

Manual и scheduled fetch фиксируют настройку вместе со снимком активной SOCKS
сессии. Stop, reconnect и отзыв owner lease продолжают отзывать этот путь.
Direct не использует настройку. Автоматического resolver и Direct fallback нет.
Настройка не управляет DNS ОС, TUN или DNS ядра и не закрывает полный P05/G-DNS.
URL подписок, токены и session-only секреты в preferences не добавляются.

Проверки (готовые бинарники, без Cargo внутри fixtures):

```
python3 crates/yonik-desktop/tests/proxy_dns_native.py /path/to/backend-test /path/to/pinned-engines
python3 crates/yonik-desktop/tests/preferences_smoke.py /path/to/yonik-desktop /path/to/evidence
```

Native fixture использует private D-Bus, ephemeral ключ, синтетическое имя
subscription.example и loopback DNS-over-TCP. Обе версии ядер берутся из
существующего proxy_native fixture; системные маршруты и DNS не изменяются.
