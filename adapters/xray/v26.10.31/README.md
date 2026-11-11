# Xray 26.10.31: закреплённый исходник и граница native интеграции

`source-lock.json` фиксирует actual upstream commit, codeload archive SHA256,
`go.mod` и `go.sum`. Это не идентичность publisher binary: её из исходников
не выводим. Существующий stock binary и registry pins не изменяются.

Сборка исходного контрольного бинарника:

```sh
/home/myrs/projects/yonik/local/orchestration/with-rust-slot \
  python tools/build_native_adapter.py --engine xray \
  --build-root /var/tmp/codex-myrs/transient/<task>/xray-build \
  --output-dir /home/myrs/projects/yonik/local/artifacts/<unique-output>
```

Сборщик удерживает BSD lock корня build root, изолирует Go caches, запрещает
изменение go.mod/go.sum и перезапись результата. До компиляции сохраняется
`build-native-adapter-logs/prebuild.json`; итоговый manifest содержит его hash.
Новых dependency edges нет. У исходного контрольного бинарника `adapter_features`
пуст, `native_protocol=null`: он не поддерживает broker/owned-TUN handshake.
Версия executable сама по себе не подтверждает поддержку native feature.

## Проверенные точки подключения

Исследован exact commit из source-lock:

- `proxy/tun/tun_linux.go:201`, `NewTun`: существующий `openFromEnv` не является
  trusted custody. Native adapter должен получать FD из закрытого worker role,
  а не из caller environment. `fdProvided` пропускает native setup; требуется
  отдельная проверка Start/Close/DNS и запрет native fallback.
- `app/dispatcher/default.go:267`, `Dispatch`; `:324`, `DispatchLink`: точки
  возврата ошибки до создания dataplane.
- `app/dispatcher/default.go:434`, `routedDispatch`: guard необходим до forced
  outbound, PickRoute и default. Ошибка FindProcess внутри matcher недостаточна:
  rule miss позволяет перейти к другому outbound.
- `common/net/find_process_linux.go` остаётся upstream реализацией в контрольной
  сборке. Будущий app-required native путь не должен вызывать её как fallback.

## Предложенный handoff Root, ещё не ABI

Inherited authenticated SOCK_SEQPACKET; HELLO/FEATURES связываются с nonce,
epoch и config digest. ATTACH принимает ровно один helper-owned TUN dup через
SCM_RIGHTS. Go владеет только dup, задаёт CLOEXEC и закрывает его при ошибке,
EOF/revoke/timeout/cancel; helper хранит original. Никаких arbitrary FD/path/env
authority и TUNSETIFF fallback. Размеры, роли, encoding и таймауты должен
подтвердить Root вместе с Rust writer до записи wire ABI.

Lookup содержит только canonical protocol/local/remote tuple IPv4/IPv6 и
session binding. KnownOwner допускается только в пределах свежего broker proof,
без локального proc lookup/cache. Unknown/Unavailable при app-required дают отказ
до routing. AND source app + destination domain/CIDR остаётся conjunction.
При отсутствии app requirements сохраняется существующая destination policy.

Требуемая матрица после handoff: TCP/UDP v4/v6 Known positive; Unknown,
unavailable, EOF, revoke, timeout, cancel, broker death, stale epoch/digest/proof;
реальный helper VM и отсутствие native setup/update/close effects. Контрольная
сборка и Python tests эту матрицу не доказывают. FR20/system_tun не объявляются.

## Per-flow DNS context

В private patch добавлен optional `dns.ContextClient.LookupIPWithContext`;
legacy `Client.LookupIP` сохранён для non-flow callers. Session routing context
передаёт исходный `context.Context` в `ResolvableContext`: managed lookup без
optional API закрывается отказом. Serial и parallel `QueryIP` получают caller
context; parallel await отслеживает отмену и не использует `WithoutCancel`.
Перед выдачей результата (включая hosts/cache) требуется новый broker proof на
стадии `AfterDNS`. Повторная выдача routing cache тоже проверяет authority.
Managed Lua DNS отклоняется: upstream script API не принимает context.

`TestNativeDNSAfterReply` использует настоящий native TCP DNS client и routing
`ResolvableContext`, controlled loopback exchanger и private query/reply.
Для serial/parallel проверены Known success, cancel, Unknown и смена epoch
между началом exchange и выпуском reply; отказные случаи дают downstream=0.
Это покрывает этот routing leaf; freedom UDP resolver и второй core требуют
отдельной реализации и приёмки. Production issuer остаётся закрытым.
