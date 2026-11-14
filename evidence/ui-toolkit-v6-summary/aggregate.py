#!/usr/bin/env python3
"""Чистая ограниченная сверка сохранённой v6; не запускает измерения."""
from __future__ import annotations

import ast
import hashlib
import json
import math
import statistics
import struct
from pathlib import Path
from typing import Any

BASE = Path('/home/myrs/projects/yonik/local')
RAW = BASE / 'evidence/ui-campaign-191-full-v6'
SNAP = BASE / 'artifacts/ui-campaign-191-20261114-v6'
GATE = BASE / 'orchestration/evidence/toolkit-v6-complete-root-gate.json'
PIN = '1b1c264be6ea80270eea0b0a23c0ac2175b3537106122aaa77fc2f668a9886a8'
OUT = Path(__file__).resolve().parent


def sha(path: Path) -> str:
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def load(path: Path) -> Any:
    check(path.stat().st_size < 2_000_000, str(path))
    return json.loads(path.read_text())


def check(value: bool, message: str) -> None:
    if not value:
        raise ValueError(message)


def quantile(values: list[float], fraction: float) -> float:
    return sorted(values)[math.ceil(len(values) * fraction) - 1]


def main() -> None:
    manifest = load(SNAP / 'manifest.json')
    check(sha(SNAP / 'manifest.json') == PIN, 'manifest pin')
    for name, digest in manifest['files'].items():
        path = SNAP / name
        check(path.resolve().is_relative_to(SNAP) and not path.is_symlink(), name)
        check(sha(path) == digest, name)
    native = load(SNAP / 'provenance/native-manifest.json')
    check(native['compiled_inputs'] == manifest['native_inputs'], 'compiled inputs')
    check(native['source_tree'] == manifest['native_build_subtree'], 'subtree ref')
    gate, terminal, binding = load(GATE), load(RAW / 'terminal.json'), load(RAW / 'binding.json')
    check(gate['terminal_sha256'] == sha(RAW / 'terminal.json'), 'terminal hash')
    check(terminal['receipt_sha256'] == sha(RAW / 'receipt.json'), 'receipt hash')
    check(gate['manifest_sha256'] == binding['manifest_sha256'] == PIN, 'binding')
    check(binding['contract'] == manifest['contract'] == terminal['run_binding']['contract'], 'contract')
    check(binding['frame_definitions'] == manifest['frame_definitions'], 'frame definitions')
    check(terminal['state'] == 'COMPLETE' and terminal['completed_cells'] == terminal['expected_cells'] == 18, 'complete')
    check(not terminal['error'] and not terminal['cleanup_errors'], 'terminal errors')
    check(len(terminal['cleanups']) == gate['cleanup_receipts'] == 19, '19 cleanups')
    for row in terminal['cleanups']:
        c = row['cleanup']
        check(c['reaped'] and c['proc_absent'] and not c['errors'] and not c['observed_descendants'], 'cleanup')
    # Из frozen source исполняются только чистый Oracle и его hash, без импортов/Collector.
    tree = ast.parse((SNAP / 'tools/ui_benchmark.py').read_text())
    nodes = [n for n in tree.body if isinstance(n, (ast.ClassDef, ast.FunctionDef)) and n.name in {'Oracle', 'page_hash', 'BenchmarkError'}]
    namespace = {'Any': Any, 'struct': struct}
    exec(compile(ast.Module(body=nodes, type_ignores=[]), '<frozen-pure-oracle>', 'exec'), namespace)
    expected = {f'rep{r}-{t}-{n}' for r in range(1, 4) for t in ('qt', 'gtk', 'slint') for n in (1000, 10000)}
    check({x['cell'] for x in gate['cells']} == expected, 'cell matrix')
    cells = []
    hashes = {str(p): sha(p) for p in [GATE, SNAP / 'manifest.json', *[RAW / n for n in ('terminal.json', 'receipt.json', 'binding.json', 'ready.json')]]}
    for item in sorted(gate['cells'], key=lambda x: x['cell']):
        path = RAW / item['cell'] / 'cell.json'
        check(sha(path) == item['sha256'], item['cell'])
        hashes[str(path)] = sha(path)
        d = load(path)
        check(d['valid'] and d['protocol_valid'] and d['reference_audit']['verified_no_references'], 'cell status')
        check(item['cell'] == f"rep{d['repetition']}-{d['toolkit']}-{d['rows']}", 'cell identity')
        check(any(x['identity'] == d['identity'] and x['cleanup'] == d['cleanup'] for x in terminal['cleanups']), 'cleanup binding')
        oracle = namespace['Oracle'](d['rows'])
        oracle.validate(d['initial_report'], 'frame')
        check(len(d['actions']) == 12 and len(d['probes']) == 6, 'protocol lengths')
        for a in d['actions']:
            oracle.validate(a['report'], oracle.apply(a['command']))
            check(a['report']['toolkit'] == d['toolkit'] and a['latency_ms'] >= 0, 'action')
        # Probe состояния: A до scroll; B после; visible до hide; hidden после hide.
        for index, probe in enumerate(d['probes']):
            replay = namespace['Oracle'](d['rows'])
            for a in d['actions'][:(0 if index == 0 else 1 if index == 1 else 10 if index < 4 else 11)]:
                replay.apply(a['command'])
            report = probe['report']
            replay.validate({k: v for k, v in report.items() if k not in ('nonce', 'rendered_frames')}, 'probe')
            check(report['nonce'] == index + 1 and probe['received_ns'] >= probe['sent_ns'], 'probe sequence')
            check(type(report['rendered_frames']) is int and report['rendered_frames'] >= 0, 'counter')
        counters = [p['report']['rendered_frames'] for p in d['probes']]
        check(counters == sorted(counters) and counters[1] > counters[0], 'canary')
        phases = []
        check([p['name'] for p in d['phases']] == ['visible', 'hidden'], 'phases')
        for i, p in enumerate(d['phases']):
            samples = p['samples']
            first, last = samples[0], samples[-1]
            elapsed = (last['monotonic_ns'] - first['monotonic_ns']) / 1e9
            check(len(samples) == 181 and p['requested_seconds'] == 900 and elapsed >= 900, 'phase coverage')
            check(p['frames_before'] == counters[2 + i * 2] and p['frames_after'] == counters[3 + i * 2], 'frame boundaries')
            check(p['frame_delta'] == p['frames_after'] - p['frames_before'], 'frame delta')
            for a, b in zip(samples, samples[1:]):
                check(b['monotonic_ns'] > a['monotonic_ns'] and b['cpu_ticks'] >= a['cpu_ticks'], 'sample order')
            for s in samples:
                check(s['pid'] == d['identity']['pid'] and s['start_ticks'] == d['identity']['start_ticks'] and not s['observed_descendants'], 'sample identity')
                check(s['clock_ticks_per_second'] == 100, 'clock ticks')
            ticks = last['cpu_ticks'] - first['cpu_ticks']
            phases.append(dict(name=p['name'], samples=len(samples), elapsed_seconds=elapsed,
                cpu_ticks=ticks, cpu_ticks_per_second=ticks / elapsed, cpu_seconds=ticks / 100,
                pss_bytes=statistics.median(s['pss_bytes'] for s in samples),
                uss_bytes=statistics.median(s['uss_bytes'] for s in samples),
                frame_delta=p['frame_delta'], wakeups=None, gpu=None, hardware_power=None,
                build_noise_samples=sum(bool(s['conditions']['observed_same_uid_builds']) for s in samples)))
        latencies = [a['latency_ms'] for a in d['actions']]
        cells.append(dict(cell=item['cell'], toolkit=d['toolkit'], rows=d['rows'], repetition=d['repetition'],
            phases=phases, action_p50_ms=quantile(latencies, .5), action_p95_ms=quantile(latencies, .95),
            actions={a['label']: a['latency_ms'] for a in d['actions']}, process_cold_start_ms=d['process_cold_start_ms']))
    groups = []
    for t in ('qt', 'gtk', 'slint'):
        for n in (1000, 10000):
            runs = [c for c in cells if c['toolkit'] == t and c['rows'] == n]
            groups.append(dict(toolkit=t, rows=n, repetitions=3,
                phases=[dict(name=name, **{k: statistics.median(c['phases'][i][k] for c in runs) for k in ('cpu_ticks', 'cpu_ticks_per_second', 'cpu_seconds', 'pss_bytes', 'uss_bytes', 'frame_delta')}) for i, name in enumerate(('visible', 'hidden'))],
                action_p50_ms=statistics.median(c['action_p50_ms'] for c in runs),
                action_p95_ms=statistics.median(c['action_p95_ms'] for c in runs),
                actions={label: dict(p50_ms=quantile([c['actions'][label] for c in runs], .5), p95_ms=quantile([c['actions'][label] for c in runs], .95), n=3) for label in runs[0]['actions']}))
    result = dict(schema_version=1, input_hashes=hashes, snapshot_files=manifest['files'], host_pins_recorded_not_reprobed=manifest['host_inputs'],
        frozen_native_commit=manifest['frozen_native_commit'], native_build_subtree=manifest['native_build_subtree'],
        checks=dict(cells=18, phases=36, cleanups=19, snapshot_files=len(manifest['files']), protocol='frozen pure Oracle replay + probe sequence/canary', raw_hashes='root gate cell hashes + terminal/receipt chain'),
        frame_definitions=manifest['frame_definitions'], binding=binding, cells=cells, medians=groups,
        repetition_ranges=[dict(toolkit=g['toolkit'], rows=g['rows'], phases=[dict(name=name, ranges={k: [min(c['phases'][i][k] for c in cells if c['toolkit']==g['toolkit'] and c['rows']==g['rows']), max(c['phases'][i][k] for c in cells if c['toolkit']==g['toolkit'] and c['rows']==g['rows'])] for k in ('cpu_ticks', 'cpu_ticks_per_second', 'pss_bytes', 'uss_bytes', 'frame_delta')}) for i, name in enumerate(('visible', 'hidden'))]) for g in groups], selection=None, gate_promotion=False)
    (OUT / 'summary.json').write_text(json.dumps(result, ensure_ascii=False, indent=2, allow_nan=False) + '\n')
    lines = ['# Сводка toolkit v6', '', 'Критерий: только программные наблюдения завершённой Xvfb-кампании; 18 ячеек, 36 фаз, 19 cleanup receipts. Повторные измерения не запускались.', '',
        'Матрица: Qt/GTK/Slint × 1000/10000 строк × 3 повтора; visible 900 с + hidden 900 с, шаг 5 с. Страница 200 строк.', '',
        'CPU: Δcpu_ticks между первым и последним sample; ticks/s = Δticks / Δmonotonic_ns × 1e9; CPU seconds = Δticks/100. PSS/USS: медиана 181 отсчёта, затем медиана трёх повторов. Память в таблице — MiB (2²⁰ байт). Xvfb исключён, учитывается только leader.', '',
        '| Toolkit | Строки | Фаза | ticks | ticks/s | CPU с | PSS MiB | USS MiB | Δ native counter |', '|---|---:|---|---:|---:|---:|---:|---:|---:|']
    for g in groups:
        for p in g['phases']:
            lines.append(f"| {g['toolkit']} | {g['rows']} | {p['name']} | {p['cpu_ticks']:g} | {p['cpu_ticks_per_second']:.6f} | {p['cpu_seconds']:.3f} | {p['pss_bytes']/2**20:.3f} | {p['uss_bytes']/2**20:.3f} | {p['frame_delta']:g} |")
    lines += ['', 'Действия: latency_ms = (ack received − send started)/1e6 из frozen runner. Для каждой ячейки p50/p95 — nearest-rank по 12 разным действиям (включая sensitivity-scroll, hide/show), затем медиана трёх таких квантилей; это смешанный набор, не распределение повторов одного действия. В JSON также есть все 12 индивидуальных latency и p50/p95 каждого label по трём повторам (p95 при n=3 = максимум).', '', '| Toolkit | Строки | Медиана action p50 мс | Медиана action p95 мс |', '|---|---:|---:|---:|']
    for g in groups:
        lines.append(f"| {g['toolkit']} | {g['rows']} | {g['action_p50_ms']:.3f} | {g['action_p95_ms']:.3f} |")
    lines += ['', 'Границы достоверности:', '',
        '- Qt: QQuickWindow.frameSwapped; GTK: GdkFrameClock.after-paint (paint cycle, не presentation); Slint: RenderingState.AfterRendering. Это разные native counters, не сопоставимый FPS. Интервалы Probe включают overhead, settling не вычтен.',
        '- X11/Xvfb, software Qt, GTK cairo, llvmpipe. Нет вывода о Wayland, реальном дисплее, GPU или физической энергии. Недоступные wakeups/GPU/power сохранены null; context switches не переименованы в wakeups.',
        f"- Фоновая сборка наблюдалась в {sum(p['build_noise_samples'] for c in cells for p in c['phases'])} из 6516 sample conditions. Это same-UID comm snapshots, не полный census нагрузки; причинная коррекция не выполнялась.",
        '- Cleanup подтверждён сохранёнными receipts и root gate, не новым осмотром процессов. Отсутствие observed descendants не исключает краткоживущий fork/exit между сканированиями.',
        '- Все 28 frozen файлов перепроверены по manifest. Host pins сохранены как исторические входы, текущие /usr и процессы не исследованы. SHA и read-only snapshot обеспечивают cooperative integrity, не adversarial sealing и не герметичную ОС.',
        '- source_tree/native_build_subtree обозначает только bench/ui subtree; frozen_native_commit — ссылка на native source, не утверждение об идентичности всего root checkout. compiled_inputs сверены с native_inputs; binary hashes — с frozen manifest. Архив исходников и checks log лишь исторические pins, отсутствующие в этом snapshot, не объявляются проверенными.',
        '- Frozen runner/doc plan не задаёт нормативного правила выбора победителя. Автоматический выбор toolkit и повышение G-UI/G-POWER не выполнены. P02 — отдельная работа.', '',
        'Точные SHA-256 всех входных JSON, каждой cell.json и frozen файлов находятся в summary.json. Raw samples не имеют самостоятельных внешних pins: их целостность покрывается SHA cell.json; фазы проверены структурно и по sample identity, monotonic order, CPU monotonicity, Probe counters и контракту.', '',
        'Воспроизведение: `python evidence/ui-toolkit-v6-summary/aggregate.py` из подготовленного worktree. Только чтение указанных входов и запись двух сводных файлов рядом со скриптом.']
    (OUT / 'report.ru.md').write_text('\n'.join(lines) + '\n')


if __name__ == '__main__':
    main()
