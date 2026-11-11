# P-04: Policy bindings, encrypted snapshots и native membership

Продолжение PR101 после frozen encoder/custody `4f75f422efe86f798c9824dcf9b1e6f9f017de6e`.
База интегрирована обычным merge `origin/main` (`90660f304e3b43e256e93eadba37f632a503ffee`).
Frozen `external_rules.rs` и `rule_set_custody.rs` не изменены.

## Исполняемый путь

1. `ResourceCatalog::build`: bounded source metadata + SHA-256 до первой
   публикации; canonical `RuleSetIndex` и существующий native encoder строятся
   из одного immutable slice. Native artifact получает собственный SHA-256 и
   публикуется через `EncryptedRuleSetStore`. Никаких загрузок или env authority.
2. `ResourceCatalog::bind`: SHA-256 полного policy preimage и exact revision,
   typed `ResourceRule::{User,Preset}` + index в ordered manifest. `BoundPolicy`
   заимствует immutable Policy; смена rules/default/private/LAN/disabled selectors
   изменяет preimage. До identity allocation проверяются legacy bounds.
3. Единственный `Policy::evaluate_with_resources` сохраняет старый control flow:
   private/LAN priority, executable AND domains AND CIDRs; inline и external
   внутри каждой категории соединены OR. Legacy `evaluate_destination` подаёт
   пустой catalog. Domain suffix lookup — по labels, CIDR lookup — по фиксированным
   33/129 prefixes, без линейного просмотра 150k записей на каждый запрос.
4. `compile_bound_policy`: existing DNS contract и native capability gates,
   resource selectors в тех же упорядоченных native rules. Xray получает
   `ext:r-<digest>.dat:YONIK`; sing-box — local source rule-set с именем из digest.
   Logical OR соединяет inline/external, logical AND соединяет категории.
   Обычный SOCKS по-прежнему отклоняет executable conditions; Names + CIDR и
   IpOnly + domains по-прежнему Unsupported. Gate не подменяется результатом PASS.
5. `bundle_digest`: domain-separated SHA-256 от config digest, policy digest,
   revision и ordered typed manifest. `RuntimeLease::seal_bundle` сверяет каждый
   ресурс и порядок с lease, затем выдаёт read-only sealed memfd. Config и
   manifest внутри compiled bundle immutable; arbitrary caller paths не принимаются.

## Store, reservations и восстановление

Trusted host передаёт directory FD. Store проверяет directory owner/0700,
берёт exclusive flock; все дочерние файлы открываются относительно FD с NOFOLLOW,
CLOEXEC, NONBLOCK и проверкой regular/owner/0600/nlink=1. Artifact filenames
выводятся из digest. Существующий artifact не перезаписывается: проверяется
AEAD + digest + size. Новые ciphertext проходят EXCL staging, file fsync,
rename NOREPLACE и directory fsync.

Encrypted `state` содержит version, generation, ordered current и previous.
Смена обоих pointers — один atomic rename после file fsync; directory fsync
завершает commit. Rollback проверяет предыдущие ресурсы и меняет pointers одним
новым generation. Ошибка записи state консервативно запрещает последующие
операции этого экземпляра: требуется reopen, в том числе при неоднозначном
результате fsync. Corrupt resources/state не исправляются и не перезаписываются.

Лимиты: manifest ≤16; source/artifact ≤128 MiB каждый; суммарно ≤1.5M records;
physical store ≤512 MiB, включая ciphertext overhead, state, retained и staging;
≤128 файлов для bounded inventory. `reservations` отдельно показывает staging,
current, previous, active/quarantine и retained без двойного physical учёта.
Whole-buffer AEAD честно ограничен размером artifact, это **не** обещание RSS.
4 MiB config, 64 KiB wire и legacy inline selector cap не повышены.

Drop runtime lease **не** освобождает его reservations: ресурс становится
quarantine. `WorkerEmptyEvidence` должен предоставлять trusted runtime после
доказанного worker **и cgroup** empty; запрос Stop таким evidence не является.
FDs не дают права prune. Пока runtime integration не разрешит quarantine,
старые/retained файлы консервативно остаются и учитываются в 512 MiB; автоматического
GC в этом slice нет. При исчерпании cap — отказ, не удаление возможного active.

## Native evidence и точные ограничения

Ignored Linux integration `actual_150k_sealed_bundle_membership` строит обоим
pinned cores 100000 distinct domains + 50000 distinct CIDR через catalog/store,
commit, binding, compiled bundle и sealed memfd. Только синтетическая controlled
fixture материализует memfd в своём временном каталоге и запускает ядра в
unprivileged user/net namespace с единственным loopback.

Xray 26.10.31 и sing-box 1.14.4 проходят native config-check, затем match/miss
отличаются по достижению controlled SOCKS upstream. Проверяются first/last record,
CIDR next-outside, exact-domain subdomain miss, inline OR external и negative controls.
Результат: 28 native-membership controls для обоих ядер; **не production Apply**.
Исходники и SHA-256 бинарников фиксируются до actual run в локальном evidence.

App AND проверен существующим evaluator; native app attribution — **Unavailable**,
подтверждённый `Unsupported(ProcessAttribution)`. Настоящий app-AND native PASS
блокируется этим capability gate и отсутствием resource FD/session integration
в worker, которым владеет W2. Для объединения domains + CIDR требуется доказанный
DNS provenance; отказ `RequiredRealResolution` сохранён. Единый DnsPolicy не
дублировался. Private IPC/worker/helper/backend/scheduler не изменялись.

Минимальный W2/root контракт: ordered manifest ≤16 без paths/bodies, config и
bundle digests, sealed resources в том же порядке, отдельные reservations и lease
до worker/cgroup-empty либо quarantine. Существующий ABI этим PR не меняется.

Evidence вне TTL: `/home/myrs/projects/yonik/local/evidence/pr101-bound/`.
48 performance и 20 snapshot прогонов не повторялись; 10x RSS/performance не заявлены.
