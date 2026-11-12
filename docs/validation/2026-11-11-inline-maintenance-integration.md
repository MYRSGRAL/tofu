# Inline Policy после maintenance/main integration

Root обычным merge объединил проверенный PR113 `0f1d944` с PR107 `adf0b71`.
В startup/nested/offline ветках сохранён generic reject_unavailable_command:
он возвращает matching routing_apply ACK и одновременно сохраняет maintenance
controller и Logs ACK. Repeated maintenance Start не получает ложный terminal ACK.
Flight revision/source fences, one-use handle и отмена до await не изменены.

Rust 1.85.1 --locked: 52 unit, 45 backend (1 прежний ignored), 5 dispatch,
3 maintenance contract — PASS. Pinned Clippy all-targets с test-support и RU/EN
проходят. Исходники записаны в source-before-113.json до проверок; после них
все hashes повторно совпали. Retained lib/backend binaries и логи сохранены
в local/validation/pr113-pr115-integration/manifest113.json.

Qt/native повторно не запускались: прежние component traces относятся к
исходному PR113, compact runtime — к PR107. Production capability остаётся
system_tun=false; положительные FR20/G-DNS/VM Apply этим merge не доказаны.
