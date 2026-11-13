# Контракт доверия package provenance (root, PR166 R2)

Решение root от 2026-11-13/14. Контракт фиксирует границы producer и verifier;
текущие материалы не являются final Linux freeze.

## Authority и TCB

Canonical Git project — /home/myrs/projects/yonik, общий .git для worktrees,
origin MYRSGRAL/yonik. Начальный reviewed anchor:
128a070884e82bf21b5c22a74ba908dadd7daf5a, принятый main. Final source выбирает
root после интеграций; candidate caller не выбирает source/revision/recipe.

Authorized producer — локальный release-controller, запущенный root или
доверенным CI job из reviewed tooling. Freeze/collect — отдельные controller
operations. Producer сам получает source archive из canonical Git, выполняет
fixed recipe на точном snapshot, собирает команды, toolchains, exit codes,
actual outputs и native builder metadata. Переданный candidate build record
не может стать trusted attestation.

Registry фиксирован:
 /home/myrs/projects/yonik/local/release-controller/RELEASE_ID/
Он выводится из canonical common Git root, не из candidate args/env.
Каталоги own UID 0700; JSON records own UID regular 0600 nlink=1,
descriptor-relative O_NOFOLLOW и BSD locks. Это не защита от malicious same
UID/root/operator. Authorized controller и такие процессы входят в TCB.
Криптоподписи, daemon, root capabilities и новая network authority не нужны.

## Candidate boundary

CLI принимает только release-id, package и inputs. Произвольные
--candidate/--expected/--repo/--build-record/--frozen-contract не поддерживаются.
Даже самосогласованный FROZEN JSON не является authority: context берётся только
из controller registry. Отсутствующий freeze/collected build → NotReady/BLOCKED.
Архивы и ELF candidate на host не исполняются.

Controller-facing функции с внедряемым runner предназначены для unit tests.
Production collect создаёт LocalRunner сам; candidate CLI не выбирает runner.
FakeTrustedRunner не создаёт production registry record.

## Core и compliance binding

Frozen adapters source locks и native codec metadata определяют версии, commits,
patches и ABI spec commit 260bc50eeb4f3a40ac97cd9a54ff0ea943a833ef.
Это полный commit спецификации, не произвольный caller integer «260».
Actual binary hashes, static ELF headers, GoBuildInfo и builder manifest
связываются с этими материалами. Patched source packages сверяются с module,
codec package и callsite hashes; upstream archives — с source locks.

Root разрешил controller-ingest результатов trusted job PR167/source158:
ce356b452675036fab3388ef64ea03f8f00910ce. Production path закреплён в tooling:
 /home/myrs/projects/yonik/local/evidence/full-pinned-158-20261113
Acceptance-context builder hash сверяется с reviewed Git blob; hashes actual
обоих binaries закреплены в controller. Ingest не является candidate option.
Перед использованием producer восстанавливает pinned core sources и применяет
frozen patches, затем проверяет GoBuildInfo/ELF/modules/codec. Это не Rust/GUI build.

Notices/licenses/source offers входят в actual package payload binding:
frozen recipe, packaging/licenses/composition, compiled notice output,
pinned core source archives и patched sources. Go dependency versions/h1 из
actual build info сопоставляются с frozen composition; соответствующие source
zip archives входят в пакет и проверяются по frozen SHA.
Существующие compliance generators/package validator переиспользуются на exact
frozen snapshot, без current working tree. Исторические #119/#147 не повторяются.

## Readiness

PASS означает только local TCB provenance preflight; final_linux_acceptance=false.
До root-selected final freeze и actual controlled build production readiness —
BLOCKED. Initial 5fd/128, retained a8 и native PR167 не являются новыми final
Linux artifacts. GUI build, host install и W2 helper integration не входят в
текущую реализацию исправлений.
