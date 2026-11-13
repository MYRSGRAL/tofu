# План декодера YNKBROK1 до frozen Rust SHA

Это design, не wire implementation и не feature acknowledgement. Основа прочитана
из `docs/specs/native-broker-bootstrap-v1.md` Rust writer; frozen commit SHA ещё
не передан. Константы/encoding/decoder пишутся только по согласованному frozen SHA.
Fixture136 не используется. Semantic validator остаётся отдельным компонентом.

## Обязательная поправка перед freeze

Root уточнил: TUN ingress LOOKUP всегда flags=0/cookie=0. Любое ненулевое
значение — Malformed; native не имеет app socket cookie proof. В прочитанном
proposal абзац LOOKUP ещё допускает bit0 has-cookie: frozen spec должен включить
новое ограничение. Reply cookie — фактическое наблюдение broker, не echo и не
фильтр запроса; иначе можно скрыть ambiguity при reuseport/socket reuse.

OWNED_TUN обязателен в selected features всегда. App-required дополнительно
требует OWNER_LOOKUP и STRICT_APP_REQUIRED. Policy flags immutable, неизвестные
bits и downgrade запрещены. Server PID/UID берутся только из trusted worker
bootstrap; HELLO, config, env и payload поля не являются источником authority.

## Чистое декодирование и состояние transport

Чистый parser получает bounded bytes, expected message type и trusted binding.
Он не открывает socket/FD, не ищет proc и не принимает caller path assertions.
Transport отдельно передаёт результат credential/pidfd проверки и custody
полученных SCM_RIGHTS. Чистый parser сам по себе не удостоверяет отправителя.

Проверки выполняются последовательно:

1. Recvmsg с MSG_CMSG_CLOEXEC; SOCK_SEQPACKET и SO_PASSCRED проверены до negotiation.
   MSG_TRUNC/MSG_CTRUNC, неизвестные/повторные ancillary records, неверные actual
   SCM_CREDENTIALS или смерть bootstrap server pidfd дают terminal отказ.
   Все уже полученные FD закрываются при любой последующей ошибке, даже если
   payload не распарсился. Буфер payload ограничен max_frame с обнаружением
   oversized frame; ancillary buffer ограничен и проверяется независимо.
2. Общий header: magic/version/type, exact total length, request id, epoch и
   runtime config digest. Header минимум 64 bytes, frame максимум 768. Нельзя
   аллоцировать по недоверенному total_bytes/path_len до проверки bounds.
3. Exact type length/reserved/nonce/role и FD count; затем semantic invariants.
   Ни один successful parse не означает READY до завершения ordered bootstrap.
4. State machine: AwaitHello → AwaitAttach → Ready; native FEATURES только после
   valid HELLO; ACK только после проверки принятого TUN identity/options.
   Forwarding и LOOKUP до ACK запрещены. Любая terminal ошибка → Retired:
   cancel context, close endpoint/TUN и все зарегистрированные TCP/UDP flows.

| Сообщение | Полная длина | SCM_RIGHTS |
| --- | ---: | ---: |
| HELLO | 176 | 0 |
| FEATURES | 112 | 0 |
| ATTACH | 128 | ровно 1 TUN FD |
| ATTACH_ACK | 104 | 0 |
| LOOKUP | 144 | 0 |
| LOOKUP_REPLY | 160 + path_len, максимум 672 | 0 |
| REVOKE | 104 | 0 |

ATTACH передаёт native только helper-owned already-bound duplicate. Никакого
отдельного TUN env FD, upstream openFromEnv, произвольного path или TUNSETIFF
fallback. Принятый FD остаётся CLOEXEC; ownership передаётся ровно одному core
handle. При ошибке до передачи закрывает transport, после передачи — core
handle через session lifecycle. Helper original остаётся у helper. Конкретный
transfer/dup API зависит от reviewed hook каждого ядра; двойное закрытие запрещено.

## Lookup, TTL и завершение flows

Один outstanding lookup; до 64 ожидающих callers, overflow → Unavailable(Limit).
ID строго растут без wrap/reuse. Reply проверяется по exact id/nonce/epoch/digest;
late reply не становится ответом следующему caller. Timeout после отправки,
EOF, revoke и broker death retire endpoint: retries и fallback запрещены.
Cancellation уже отправленного lookup тоже retire endpoint, чтобы не оставлять
поздний reply в очереди. Отмена до отправки не создаёт request id на проводе.

Для каждого lookup фиксируются monotonic send/receive instants. TTL ответа
не больше 20ms; из него вычитается полный RTT с saturating arithmetic. Нулевой
остаток → Unavailable. Новый Evidence не получает времени сверх broker proof.
Guard повторяется после DNS и непосредственно перед новым dial. Сессия отменяет
pending DNS/dial через context; Track проверяет proof ещё раз перед публикацией
соединения. Revoke закрывает все зарегистрированные TCP и UDP flows, а не только
TUN FD. READY/ACK и трафик не продлевают helper anchor lease.

Known требует согласованных reason/owner/path/TTL полей по frozen schema.
Unknown/Unavailable требуют нулевых owner fields/cookie/TTL и path_len=0.
Path проверяется на exact bounded UTF8/no NUL; это только broker-observed kernel
attribute, не native proc guess. Known не означает policy allow. Существующий
matcher сохраняет AND source app + destination domain/CIDR и destination/private/
LAN semantics; app-required Unknown/Unavailable дают отказ до default dispatch.

## Проверки после frozen SHA

- Golden bytes от Rust writer для каждого type и каждого отказа; round-trip
  encoder не является единственным oracle. Exact spec SHA включается в manifest.
- Все truncation prefixes, oversized payload/path, unknown bits/types/roles,
  reserved bytes, несовпадающие id/nonce/epoch/digest, invalid UTF8/NUL.
- FD count 0/1/2, лишний SCM_RIGHTS у каждого non-ATTACH type, CLOEXEC и закрытие
  всех received FD на malformed payload/credentials/timeout/cancel.
- TUN ingress flags/cookie ненулевые → Malformed; reply cookie различается при
  одинаковом tuple и не превращается в request filter; ambiguous → deny.
- Полная feature matrix OWNED_TUN/OWNER_LOOKUP/STRICT_APP_REQUIRED/app-required;
  old stock/native asset → Unsupported без silent legacy fallback.
- Ordered bootstrap deadline 1s; lookup RTT/TTL boundaries 20ms, queue overflow,
  revoke/EOF/death/late reply, отсутствие retry и stale proof/cache.
- Затем reviewed actual hooks обоих ядер, isolated TUN/flow и real helper VM
  matrix. До этого native_features={} и native_protocol=null; FR20 и global
  system_tun не заявляются. Существующий compiled source control не новый pin.
