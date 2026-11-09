# DNS provenance and direct resolution pin for sing-box v1.14.4

This source patch chain applies only to upstream commit
`c005f35300ce76427dc285481ec5695f5b179cd2`. It adds `strict_complete: true`
to the route `resolve` action. An error from either required family rejects the
combined result; a successful NODATA response is not a transport failure.
Default/false and single-family lookup behavior remain compatible.

Apply strict-complete-dns, resolution-provenance and resolution-pin patches in
that order; the builder does this and binds every digest in its manifest.
`pin_resolution: true` on `resolve` requires original TTL provenance and binds
native direct TCP/UDP to the resulting address set. Synthetic/stale/unproven sources
fail explicitly. Proxy/custom handlers are unsupported in this mode.

This is an experimental integration component. It does not implement trusted network/policy
leases, safe Fake-IP reuse, system DNS ownership, TUN or full G-DNS.
The yonik compiler keeps mixed DNS/Fake-IP disabled. The application and policy
remain Rust; this patch changes only native engine DNS result handling.

Build and run the isolated positive/negative controls from the repository root:

```sh
python tools/build_native_adapter.py --output-dir "$PWD/local/artifacts/sing-box-adapter-v1.14.4"
python tools/resolution_pin_smoke.py local/artifacts/sing-box-adapter-v1.14.4/sing-box local/engines/sing-box-1.14.4/sing-box --manifest local/artifacts/sing-box-adapter-v1.14.4/manifest.json --output local/validation/resolution-pin/native
```

The builder locks its task directory, verifies the pinned source and canonical
patch, isolates Go caches, preserves go.mod/go.sum and records build settings and
binary SHA-256. It requires a compatible local Go toolchain and does not install
one. Existing output and interrupted builds are refused for manual inspection.
The smoke launcher always creates user/network/PID/mount namespaces. It never
changes host routes, DNS or firewall. Use a fresh evidence output directory.

The manifest describes the experimental binary; it is not a signed capability
attestation and `strict_dns_gate` remains false. Build tags follow upstream's
`release/DEFAULT_BUILD_TAGS_OTHERS`. Bit-for-bit reproducibility has not been proved.
Upstream source and license: [sing-box v1.14.4](https://github.com/SagerNet/sing-box/tree/v1.14.4),
[LICENSE](https://github.com/SagerNet/sing-box/blob/v1.14.4/LICENSE).

## По-русски

Патч добавляет `strict_complete: true` в route action `resolve` закреплённого
sing-box. При ошибке любого обязательного семейства частичный A/AAAA не становится
успешным результатом. Успешный NODATA допустим; default/false и односемейный lookup
сохраняют прежнее поведение. Ошибка строгого объединения не скрывается legacy
fallback. Внутренние speculative `evaluate` DNS DSL этим флагом не запрещаются;
проверяется итог каждого обязательного семейства.

Это экспериментальная интеграция, а не готовый строгий DNS. Полная lease с сетевыми/policy epochs, lifecycle Fake-IP, владение системным DNS
и TUN ещё не реализованы.
Основная часть yonik остаётся на Rust. Сборка не заменяет используемый приложением
официальный бинарник и не включает смешанную DNS/Fake-IP capability.

Цепочка дополнительно сохраняет provenance исходного ответа и через
`pin_resolution` проверяет адреса/TTL native direct TCP/UDP. Сетевые/policy epochs,
приватный путь, proxy/Xray parity и lifecycle Fake-IP остаются открытыми.

Команды выше проверяют источник и канонический patch, сохраняют manifest и запускают
изолированную нативную матрицу с официальным контролем. Build root удерживается
`flock`, Go cache размещается внутри него, output сохраняется вне TTL. Повторная
сборка поверх частичного результата и перезапись output запрещены. Manifest не
является подписанным доказательством capability. Подробные результаты и ограничения:
[проверки](../../../docs/validation/2026-11-09-strict-complete-adapter.md).

[Provenance/direct pin evidence and limits / Проверки и ограничения](../../../docs/validation/2026-11-09-resolution-pin.md).
