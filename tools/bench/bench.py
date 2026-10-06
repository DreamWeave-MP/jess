#!/usr/bin/env python3
"""jess's benchmark harness.

    python3 tools/bench/bench.py showcase [--runs 5] [--only tes3merge ...]
    python3 tools/bench/bench.py canaries [--runs 5] [--scale 10] [--check BASELINE]
    python3 tools/bench/bench.py compare BASELINE CANDIDATE
    python3 tools/bench/bench.py docs RESULTS
    python3 tools/bench/bench.py graph RESULTS

`showcase` runs Dreamwind's load-order tools over the real load order, next to the native tools
they port, and checks that both wrote the same files. `canaries` runs small synthetic workloads
that catch runtime and binding regressions without game data. Each writes one JSON results file
under target/bench/results. `compare` puts two results files side by side, `docs` turns a showcase
results file into the numbers the site prints, and `graph` draws the showcase.

Standard library only (Python 3.11+), Linux: CPU time and peak memory come from wait4's rusage.
Everything the harness writes goes under target/bench; the OpenMW config it reads is only read.
"""

from __future__ import annotations

import argparse
import dataclasses
import datetime
import hashlib
import json
import os
import platform
import re
import shutil
import statistics
import struct
import subprocess
import sys
import time
import tomllib
from pathlib import Path
from typing import NoReturn

ROOT = Path(__file__).resolve().parents[2]
TOOLS = ROOT / 'tools' / 'bench'
# The package the load-order tools come from, activated with `jess run --package`.
DREAMWIND = ROOT / 'jpk' / 'dreamwind'
WORK = ROOT / 'target' / 'bench'
SITE_DATA = ROOT / 'site' / 'data' / 'benchmarks.toml'

# Merged Lands has no release binary for Linux; the harness builds this commit of it (0.2.0).
MERGED_LANDS_REPOSITORY = 'https://github.com/Sophialovesmorrowind/OpenMWMergedLands'
MERGED_LANDS_COMMIT = '103cf66e02491b62becbc14c8bb98653f56e51ef'

# The tools' own outputs never go into the benchmark's load order.
OUTPUT_PLUGINS = ('Merged Objects.esp', 'S3LightFixes.omwaddon', 'Merged Lands.omwaddon')

TES3MERGE_INI = """[General]
PauseOnCompletion = false
DumpMergedRecordsToLog = false
TextEncodingCode = 1252
InstallPath = {dir}

[RecordTypes]
""" + ''.join(
    f'{tag} = true\n'
    for tag in (
        'ACTI ALCH APPA ARMO BODY BOOK BSGN CELL CLAS CLOT CONT CREA DOOR ENCH GMST INGR LEVC LEVI '
        'LIGH LOCK MGEF MISC NPC_ PROB RACE REPA SKIL SNDG SOUN SPEL STAT WEAP'
    ).split()
)


@dataclasses.dataclass(frozen=True)
class Known:
    """A difference from the native tool's output that's understood, and why.

    `normalize` names byte rules applied to both files before comparing records; `native_only`
    lists (tag, id) records only the native tool writes. Anything else that differs fails.
    """

    reason: str
    normalize: tuple[str, ...] = ()
    native_only: tuple[tuple[str, str], ...] = ()


@dataclasses.dataclass(frozen=True)
class Workload:
    name: str
    title: str
    # The Dreamwind tool modules the run's driver calls with `(state, openmw)`, in order.
    tools: tuple[str, ...]
    # Files compared and hashed, relative to the run folder.
    outputs: tuple[str, ...]
    # Luau the driver runs before the tools, to set `state`.
    options: str | None = None
    # The native tool's command; `{dir}` is the run folder, `{tool}` its binary.
    native: tuple[str, ...] | None = None
    native_title: str | None = None
    # Files written into the run folder before every run, both implementations.
    seed: tuple[tuple[str, str], ...] = ()
    known: tuple[tuple[str, tuple[Known, ...]], ...] = ()


WORKLOADS = (
    Workload(
        name='tes3merge',
        title='TES3Merge',
        tools=('@dreamwind/tools/tes3merge',),
        outputs=('data-local/Merged Objects.esp',),
        native=('{tool}',),
        native_title='TES3Merge',
        seed=(('TES3Merge.ini', TES3MERGE_INI),),
        known=(
            (
                'data-local/Merged Objects.esp',
                (
                    Known(
                        reason='native TES3Merge merges the garbage in AIDT padding: it writes four creature '
                        'records that change nothing, and copies the padding into the ones both write',
                        normalize=('aidt-padding',),
                        native_only=(
                            ('CREA', 'dagoth endus'),
                            ('CREA', 'centurion_fire_dead'),
                            ('CREA', 'golden saint_staada'),
                            ('CREA', 'atronach_frost_summon'),
                        ),
                    ),
                    Known(
                        reason="native TES3Merge writes MGEF subrecords in TES3Lib's property order, sounds after "
                        'the description, not the order of the Construction Set, OpenMW, and every source plugin; '
                        'the reordered bytes differ from the original, so it writes three magic effects that '
                        'change nothing',
                        normalize=('mgef-subrecord-order',),
                        native_only=(('MGEF', '64'), ('MGEF', '123'), ('MGEF', '127')),
                    ),
                    Known(
                        reason="native TES3Merge keeps a cell's NAM0, the count of the temporary references "
                        'after it, when it comes before the first reference, and then leaves every reference '
                        "out; the port writes no count for references it doesn't write, and OpenMW's engine "
                        'never reads it',
                        normalize=('cell-nam0',),
                    ),
                ),
            ),
        ),
    ),
    Workload(
        name='s3lightfixes',
        title='S3LightFixes',
        tools=('@dreamwind/tools/s3lightfixes',),
        options='state.s3lightfixes = { noNotifications = true }\n',
        outputs=('data-local/S3LightFixes.omwaddon', 'lightconfig.toml', 'lightconfig.log'),
        native=('{tool}', '--openmw-cfg', '{dir}', '--no-notifications'),
        native_title='S3LightFixes',
    ),
    Workload(
        name='s3lightfixes-sun',
        title='S3LightFixes\ninterior sun off',
        tools=('@dreamwind/tools/s3lightfixes',),
        outputs=('data-local/S3LightFixes.omwaddon', 'lightconfig.log'),
        native=('{tool}', '--openmw-cfg', '{dir}', '--no-notifications'),
        native_title='S3LightFixes',
        seed=(('lightconfig.toml', 'disable_interior_sun = true\nno_notifications = true\n'),),
    ),
    Workload(
        name='mergedlands',
        title='Merged Lands',
        tools=('@dreamwind/tools/merged-lands',),
        outputs=('data-local/Merged Lands.omwaddon',),
        native=('{tool}', '--openmw-cfg', '{dir}', '--config-dir', '{dir}', '--merged-lands-dir', '{dir}'),
        native_title='Merged Lands',
    ),
    Workload(name='doctor', title='Doctor', tools=('@dreamwind/tools/doctor',), outputs=('jess-doctor.yaml',)),
    Workload(
        name='doctor-assets',
        title='Doctor, assets and meshes',
        tools=('@dreamwind/tools/doctor',),
        options="state.doctor = { checks = { 'assets', 'meshes' } }\n",
        outputs=('jess-doctor.yaml',),
    ),
    Workload(
        name='doctor-rest',
        title='Doctor, without assets and meshes',
        tools=('@dreamwind/tools/doctor',),
        options="state.doctor = { checks = { 'masters', 'content', 'dirt', 'conflicts' } }\n",
        outputs=('jess-doctor.yaml',),
    ),
)

JESS_COMMAND = ('{tool}', 'run', '--package', '{package}', '{dir}/bench/run.luau', '{dir}/openmw.cfg')
CANARIES = sorted(path.stem for path in (TOOLS / 'canaries').glob('*.luau') if path.stem != 'setup')


def fail(message: str) -> NoReturn:
    raise SystemExit(f'bench: {message}')


# Context: what ran, where, and on what.


def git(*args: str) -> str:
    return subprocess.run(('git', *args), cwd=ROOT, check=True, capture_output=True, text=True).stdout.strip()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open('rb') as file:
        while block := file.read(1 << 20):
            digest.update(block)
    return digest.hexdigest()


def jess_context(binary: Path, runtime: str | None) -> dict:
    version = subprocess.run((str(binary), '--version'), check=True, capture_output=True, text=True).stdout
    if 'unverified toolchain' in version:
        fail(
            f'{binary} was built without cross-language LTO (RUSTFLAGS or L3I_UNVERIFIED_TOOLCHAIN was set); '
            'rebuild it with neither to measure it'
        )
    lock = tomllib.loads((ROOT / 'Cargo.lock').read_text())
    versions = {package['name']: package['version'] for package in lock['package']}
    # The definitions' first line names the Luau the binary runs; `_VERSION` is just "Luau".
    header = subprocess.run((str(binary), 'luau-types'), check=True, capture_output=True, text=True).stdout
    luau = re.search(r'for (Luau \S+)', header.partition('\n')[0])
    return {
        'commit': git('rev-parse', 'HEAD'),
        'dirty': bool(git('status', '--porcelain', '--untracked-files=no')),
        'binary': str(binary.relative_to(ROOT)) if binary.is_relative_to(ROOT) else binary.name,
        'binary_sha256': sha256(binary),
        'profile': binary.parent.name,
        'runtime': runtime or f'l3i {versions["l3i"]}',
        'luau': luau.group(1) if luau else 'Luau',
    }


def read_key_values(path: Path, separator: str) -> dict[str, str]:
    values = {}
    for line in path.read_text().splitlines():
        key, found, value = line.partition(separator)
        if found:
            values[key.strip()] = value.strip().strip('"')
    return values


def machine_context() -> dict:
    cpu = read_key_values(Path('/proc/cpuinfo'), ':').get('model name', platform.processor())
    memory = read_key_values(Path('/proc/meminfo'), ':')['MemTotal'].split()[0]
    governor = Path('/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor')
    return {
        'os': read_key_values(Path('/etc/os-release'), '=').get('PRETTY_NAME', platform.system()),
        'kernel': platform.release(),
        'cpu': cpu,
        'threads': os.cpu_count(),
        'memory_bytes': int(memory) * 1024,
        'governor': governor.read_text().strip() if governor.exists() else None,
    }


def busiest_processes() -> list[str]:
    """The five processes with the most CPU, as ps counts it (averaged over each one's life)."""
    output = subprocess.run(
        ('ps', '-eo', 'pid,pcpu,comm', '--sort=-pcpu', '--no-headers'), capture_output=True, text=True
    ).stdout
    busiest = []
    for line in output.splitlines():
        pid, cpu, name = line.split(None, 2)
        if int(pid) != os.getpid():
            busiest.append(f'{cpu}% {name}')
    return busiest[:5]


def wait_until_quiet(max_load: float | None, patience_minutes: float) -> None:
    if max_load is None:
        return
    deadline = time.monotonic() + patience_minutes * 60
    while (load := os.getloadavg()[0]) > max_load:
        if time.monotonic() > deadline:
            print(f'bench: load is still {load:.2f} after {patience_minutes:g} minutes; measuring anyway', flush=True)
            return
        print(f'bench: load {load:.2f} > {max_load:g}; busiest: {busiest_processes()[:3]}; waiting', flush=True)
        time.sleep(120)


# Measuring.


def isolated_environment() -> dict[str, str]:
    """The environment every measured process gets: the user's, minus anything that points jess or
    OpenMW at a config, with a home folder of its own so no tool finds or writes the real one."""
    home = WORK / 'home'
    (home / '.config').mkdir(parents=True, exist_ok=True)
    environment = {key: value for key, value in os.environ.items() if not key.startswith(('JESS_', 'OPENMW_'))}
    environment['HOME'] = str(home)
    environment['XDG_CONFIG_HOME'] = str(home / '.config')
    return environment


def measure(command: list[str], cwd: Path, environment: dict[str, str], log: Path) -> dict:
    with log.open('wb') as output:
        started = time.perf_counter()
        process = subprocess.Popen(
            command, cwd=cwd, env=environment, stdin=subprocess.DEVNULL, stdout=output, stderr=subprocess.STDOUT
        )
        _, status, usage = os.wait4(process.pid, 0)
        wall = time.perf_counter() - started
    process.returncode = os.waitstatus_to_exitcode(status)
    if process.returncode != 0:
        tail = log.read_text(errors='replace').splitlines()[-5:]
        fail(f'{" ".join(command)} exited with {process.returncode}; {log}:\n' + '\n'.join(tail))
    return {
        'wall': wall,
        'cpu': usage.ru_utime + usage.ru_stime,
        'user': usage.ru_utime,
        'system': usage.ru_stime,
        # Linux reports ru_maxrss in KiB.
        'rss': usage.ru_maxrss * 1024,
    }


def summarize(samples: list[dict], key: str) -> dict:
    values = [sample[key] for sample in samples]
    return {'median': statistics.median(values), 'min': min(values), 'max': max(values)}


def summary(samples: list[dict]) -> dict:
    return {key: summarize(samples, key) for key in ('wall', 'cpu', 'rss')}


def write_results(results: dict) -> Path:
    folder = WORK / 'results'
    folder.mkdir(parents=True, exist_ok=True)
    stamp = datetime.datetime.now(datetime.UTC).strftime('%Y%m%d-%H%M%S')
    path = folder / f'{stamp}-{results["kind"]}.json'
    path.write_text(json.dumps(results, indent=1) + '\n')
    return path


# TES3 plugins: records, the header's timestamp, and known differences.

# TES3Merge writes an ISO 8601 time, Merged Lands seconds since 1970 (`Generated at 1790548496 UTC.`).
TIMESTAMP = re.compile(rb'\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z|(?<=Generated at )\d+(?= UTC)')
# The TES3 record's HEDR: its 16-byte record header, then HEDR's own 8, version, flags, and author.
DESCRIPTION = slice(16 + 8 + 4 + 4 + 32, 16 + 8 + 4 + 4 + 32 + 256)
RECORD_COUNT = slice(DESCRIPTION.stop, DESCRIPTION.stop + 4)


def is_plugin(data: bytes) -> bool:
    return data[:4] == b'TES3' and data[16:20] == b'HEDR'


def mask_timestamp(data: bytes) -> bytes:
    """The plugin with the timestamp in its header's description blanked, and nothing else."""
    description = TIMESTAMP.sub(lambda match: b'#' * len(match[0]), data[DESCRIPTION])
    return data[: DESCRIPTION.start] + description + data[DESCRIPTION.stop :]


def subrecords(body: bytes):
    position = 0
    while position < len(body):
        tag, size = body[position : position + 4], struct.unpack_from('<I', body, position + 4)[0]
        yield tag, position + 8, size
        position += 8 + size


def record_key(tag: bytes, body: bytes) -> tuple[str, str]:
    """A record's type and what identifies it: its NAME, INDX, or INTV (with the grid for a CELL)."""
    identity = ''
    for subtag, start, size in subrecords(body):
        payload = body[start : start + size]
        if subtag in (b'NAME', b'INAM') and not identity:
            identity = payload.split(b'\0')[0].decode('latin-1')
            if tag != b'CELL':
                break
        elif subtag in (b'INDX', b'INTV') and not identity:
            identity = str(struct.unpack_from('<i', payload)[0]) if size == 4 else str(struct.unpack('<2i', payload))
            break
        elif subtag == b'DATA' and tag == b'CELL' and size == 12:
            flags, x, y = struct.unpack('<I2i', payload)
            if not flags & 1:
                identity += f' ({x}, {y})'
            break
    return tag.decode('latin-1'), identity


def normalize_aidt_padding(tag: bytes, body: bytearray) -> None:
    # AIDT: hello (2), fight, flee, alarm, three bytes the Construction Set never cleared, services (4).
    if tag not in (b'CREA', b'NPC_'):
        return
    for subtag, start, size in subrecords(bytes(body)):
        if subtag == b'AIDT' and size == 12:
            body[start + 5 : start + 8] = b'\0\0\0'


# The Construction Set's MGEF subrecord order, which OpenMW's `MagicEffect::save` writes too.
MGEF_ORDER = {
    tag: rank
    for rank, tag in enumerate(
        (b'INDX', b'MEDT', b'ITEX', b'PTEX', b'BSND', b'CSND', b'HSND', b'ASND')
        + (b'CVFX', b'BVFX', b'HVFX', b'AVFX', b'DESC')
    )
}


def normalize_mgef_subrecord_order(tag: bytes, body: bytearray) -> None:
    if tag != b'MGEF':
        return
    chunks = [(subtag, bytes(body[start - 8 : start + size])) for subtag, start, size in subrecords(bytes(body))]
    chunks.sort(key=lambda chunk: MGEF_ORDER.get(chunk[0], len(MGEF_ORDER)))
    body[:] = b''.join(chunk for _, chunk in chunks)


def normalize_cell_nam0(tag: bytes, body: bytearray) -> None:
    # NAM0 counts the temporary references after it: in a CELL without references it counts nothing.
    if tag != b'CELL':
        return
    chunks = [(subtag, bytes(body[start - 8 : start + size])) for subtag, start, size in subrecords(bytes(body))]
    if any(subtag in (b'FRMR', b'MVRF') for subtag, _ in chunks):
        return
    body[:] = b''.join(chunk for subtag, chunk in chunks if subtag != b'NAM0')


NORMALIZERS = {
    'aidt-padding': normalize_aidt_padding,
    'mgef-subrecord-order': normalize_mgef_subrecord_order,
    'cell-nam0': normalize_cell_nam0,
}


def plugin_records(data: bytes, normalize: tuple[str, ...]) -> tuple[bytes, list[tuple[tuple[str, str], bytes]]]:
    """The header record (timestamp and record count masked) and every other record by key."""
    records, position = [], 0
    header = b''
    while position < len(data):
        tag, size = data[position : position + 4], struct.unpack_from('<I', data, position + 4)[0]
        raw = data[position : position + 16 + size]
        position += 16 + size
        if tag == b'TES3':
            header = mask_timestamp(raw)
            header = header[: RECORD_COUNT.start] + b'\0' * 4 + header[RECORD_COUNT.stop :]
            continue
        body = bytearray(raw[16:])
        for rule in normalize:
            NORMALIZERS[rule](tag, body)
        # A rule that drops a subrecord changes the size the record header gives.
        header = raw[:4] + struct.pack('<I', len(body)) + raw[8:16]
        records.append((record_key(tag, bytes(body)), header + bytes(body)))
    return header, records


def differing_subrecords(left: bytes, right: bytes) -> list[str]:
    """What two copies of a record differ in: their header flags, which subrecords they have, or
    which of the same subrecords differ."""
    if left[8:16] != right[8:16]:
        return ['record flags']
    lefts = [
        (tag.decode('latin-1'), left[16 + start : 16 + start + size]) for tag, start, size in subrecords(left[16:])
    ]
    rights = [
        (tag.decode('latin-1'), right[16 + start : 16 + start + size]) for tag, start, size in subrecords(right[16:])
    ]
    left_tags, right_tags = [tag for tag, _ in lefts], [tag for tag, _ in rights]
    if left_tags != right_tags:
        return [f'subrecords {" ".join(left_tags)} against {" ".join(right_tags)}']
    return [tag for (tag, a), (_, b) in zip(lefts, rights, strict=True) if a != b]


def compare_file(jess: Path, native: Path, known: tuple[Known, ...]) -> dict:
    jess_data, native_data = jess.read_bytes(), native.read_bytes()
    if jess_data == native_data:
        return {'status': 'identical'}
    if not (is_plugin(jess_data) and is_plugin(native_data)):
        return {'status': 'mismatch', 'details': first_difference(jess_data, native_data)}
    if mask_timestamp(jess_data) == mask_timestamp(native_data):
        return {'status': 'identical-except-timestamp'}

    rules = tuple(rule for difference in known for rule in difference.normalize)
    jess_header, jess_records = plugin_records(jess_data, rules)
    native_header, native_records = plugin_records(native_data, rules)
    jess_keys, native_keys = dict(jess_records), dict(native_records)
    only_native = [key for key, _ in native_records if key not in jess_keys]
    only_jess = [key for key, _ in jess_records if key not in native_keys]
    differing = [
        f'{tag} {identity}: {", ".join(differing_subrecords(raw, native_keys[tag, identity]))}'
        for (tag, identity), raw in jess_records
        if (tag, identity) in native_keys and native_keys[tag, identity] != raw
    ]
    common_jess = [key for key, _ in jess_records if key in native_keys]
    common_native = [key for key, _ in native_records if key in jess_keys]
    expected = {key for difference in known for key in difference.native_only}

    problems = {}
    if jess_header != native_header:
        problems['header'] = 'differs beyond its timestamp and record count'
    if set(only_native) != expected:
        problems['only_native'] = [list(key) for key in only_native]
    if only_jess:
        problems['only_jess'] = [list(key) for key in only_jess]
    if differing:
        problems['differing'] = differing
    if common_jess != common_native:
        problems['order'] = 'the records both write are in a different order'
    if len(jess_keys) != len(jess_records) or len(native_keys) != len(native_records):
        problems['duplicates'] = 'a record appears twice in one of them'
    if problems or not known:
        return {'status': 'mismatch', 'details': problems or "only the header's record count differs"}
    return {
        'status': 'known-differences',
        'details': {
            'reasons': [difference.reason for difference in known],
            'normalized': list(rules),
            'only_native': [list(key) for key in only_native],
        },
    }


def first_difference(left: bytes, right: bytes) -> str:
    offset = next(
        (index for index, (a, b) in enumerate(zip(left, right, strict=False)) if a != b), min(len(left), len(right))
    )
    line = left[:offset].count(b'\n') + 1
    return f'first difference at byte {offset} (line {line}); {len(left)} and {len(right)} bytes'


def describe_output(path: Path) -> dict:
    data = path.read_bytes()
    described = {'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()}
    if is_plugin(data):
        described['normalized_sha256'] = hashlib.sha256(mask_timestamp(data)).hexdigest()
        records: dict[str, int] = {}
        for (tag, _), _ in plugin_records(data, ())[1]:
            records[tag] = records.get(tag, 0) + 1
        described['records'] = records
    return described


# The showcase.


def quoted(path: Path) -> str:
    text = str(path).replace('&', '&&').replace('"', '&"')
    return f'"{text}"'


def merged_lands_binary() -> Path:
    source = WORK / 'src' / 'mergedlands'
    binary = source / 'target' / 'release' / 'merged_lands'
    if binary.exists():
        return binary
    print(f'bench: building Merged Lands {MERGED_LANDS_COMMIT[:7]} in {source}', flush=True)
    if not source.exists():
        subprocess.run(('git', 'clone', '--quiet', MERGED_LANDS_REPOSITORY, str(source)), check=True)
    subprocess.run(('git', '-C', str(source), 'checkout', '--quiet', MERGED_LANDS_COMMIT), check=True)
    # It lives inside jess's workspace folder; an empty [workspace] keeps it its own.
    manifest = source / 'Cargo.toml'
    if '\n[workspace]' not in manifest.read_text():
        manifest.write_text(manifest.read_text() + '\n[workspace]\n')
    subprocess.run(('cargo', 'build', '--release', '--quiet'), cwd=source, check=True)
    return binary


def flatten(jess: Path, config: str | None, folder: Path) -> dict:
    """Writes the load order as one standalone config and returns its facts."""
    folder.mkdir(parents=True, exist_ok=True)
    command = [str(jess), 'run', str(TOOLS / 'flatten.luau'), str(folder), config or '', *OUTPUT_PLUGINS]
    subprocess.run(command, check=True)

    base = (folder / 'openmw.cfg.in').read_text()
    content, sizes = [], []
    for line in (folder / 'content.tsv').read_text().splitlines():
        name, _, path = line.partition('\t')
        content.append(name)
        if path and not name.lower().endswith('.omwscripts'):
            sizes.append(Path(path).stat().st_size)
    return {
        'config': config or 'OpenMW default',
        'config_sha256': hashlib.sha256(base.encode()).hexdigest(),
        'content_files': len(content),
        'plugins': len(sizes),
        'plugin_bytes': sum(sizes),
        'data_dirs': base.count('\ndata=') + base.startswith('data='),
        'base': base,
    }


def prepare(run: Path, workload: Workload, base: str, implementation: str) -> None:
    """A fresh run folder: the config, the seeds, and for jess the driver that loads the config and
    calls the tools."""
    shutil.rmtree(run, ignore_errors=True)
    (run / 'data-local').mkdir(parents=True)
    lines = [base.rstrip('\n'), f'data-local={quoted(run / "data-local")}']
    (run / 'openmw.cfg').write_text('\n'.join(lines) + '\n')
    if implementation != 'native':
        bench = run / 'bench'
        bench.mkdir()
        driver = ['local config: string = ...', 'local openmw = jess.openmw.load(config)', 'local state = {}', '']
        if workload.options:
            driver.append(workload.options.rstrip('\n'))
        driver += [f"require('{tool}')(state, openmw)" for tool in workload.tools]
        if implementation == 'facts':
            shutil.copy(TOOLS / 'facts.luau', bench)
            driver.append("require('./facts')(state)")
        (bench / 'run.luau').write_text('\n'.join(driver) + '\n')
    for name, text in workload.seed:
        (run / name).write_text(text.format(dir=run))


def run_workload(workload: Workload, jess: Path, tool: Path | None, base: str, environment: dict, arguments) -> dict:
    folder = WORK / 'showcase' / workload.name
    run = folder / 'run'
    commands = {'jess': [part.format(tool=jess, dir=run, package=DREAMWIND) for part in JESS_COMMAND]}
    if tool:
        commands['native'] = [part.format(tool=tool, dir=run) for part in workload.native]

    load = os.getloadavg()
    samples = {implementation: [] for implementation in commands}
    rounds = arguments.warmup + arguments.runs
    for round in range(rounds):
        # Alternating keeps drift on a long run from favoring either one.
        for implementation, command in commands.items():
            prepare(run, workload, base, implementation)
            sample = measure(command, run, environment, folder / f'{implementation}.log')
            print(f'  {workload.name} {implementation} {round + 1}/{rounds}: {sample["wall"]:.3f} s', flush=True)
            if round < arguments.warmup:
                continue
            samples[implementation].append(sample)
            if round == rounds - 1:
                keep = folder / implementation
                shutil.rmtree(keep, ignore_errors=True)
                keep.mkdir()
                for output in workload.outputs:
                    shutil.copy2(run / output, keep / Path(output).name)

    # One more, unmeasured, to record what the script reports about the load order.
    prepare(run, workload, base, 'facts')
    measure(commands['jess'], run, environment, folder / 'facts.log')
    facts = {}
    for line in (folder / 'facts.log').read_text().splitlines():
        # Counts only: the rest (paths, flags) says nothing about the load order.
        if line.startswith('fact '):
            _, name, value = line.split(' ', 2)
            if value.isdigit():
                facts[name] = int(value)

    implementations = {}
    for implementation in commands:
        implementations[implementation] = {
            'title': 'jess' if implementation == 'jess' else workload.native_title,
            'version': None if implementation == 'jess' else native_version(tool, folder / 'native.log'),
            # The command as run, with the run folder and native binary left as placeholders.
            'command': list(JESS_COMMAND if implementation == 'jess' else workload.native),
            'samples': samples[implementation],
            'summary': summary(samples[implementation]),
            'outputs': {
                output: describe_output(folder / implementation / Path(output).name) for output in workload.outputs
            },
        }

    equivalence = {}
    if tool:
        known = dict(workload.known)
        for output in workload.outputs:
            name = Path(output).name
            equivalence[output] = compare_file(folder / 'jess' / name, folder / 'native' / name, known.get(output, ()))

    return {
        'name': workload.name,
        'title': workload.title,
        'load_start': load,
        'implementations': implementations,
        'equivalence': equivalence,
        'facts': facts,
    }


VERSION = re.compile(r'\d+\.\d+\.\d+(?:\.\d+)?')


def native_version(tool: Path, log: Path) -> str | None:
    """The version the native tool printed in its log, or else says with --version."""
    match = VERSION.search(log.read_text(errors='replace')[:4096])
    if not match:
        output = subprocess.run((str(tool), '--version'), cwd=log.parent, capture_output=True, text=True).stdout
        match = VERSION.search(output)
    return match[0] if match else None


def native_tools(arguments, selected: list[Workload]) -> dict[str, Path | None]:
    if arguments.no_native:
        return {}
    names = {workload.name for workload in selected if workload.native}
    tools: dict[str, Path | None] = {}
    for name in names:
        if name == 'tes3merge':
            tools[name] = arguments.tes3merge
        elif name.startswith('s3lightfixes'):
            tools[name] = arguments.s3lightfixes
        elif name == 'mergedlands':
            tools[name] = arguments.merged_lands or merged_lands_binary()
    for name, tool in tools.items():
        if not tool or not Path(tool).exists():
            fail(f'no native binary for {name}; pass it, or --no-native')
        tools[name] = Path(tool).resolve()
    return tools


def showcase(arguments) -> int:
    jess = Path(arguments.jess).resolve()
    WORK.mkdir(parents=True, exist_ok=True)
    selected = [workload for workload in WORKLOADS if not arguments.only or workload.name in arguments.only]
    tools = native_tools(arguments, selected)
    load_order = flatten(jess, arguments.openmw_cfg, WORK / 'config')
    base = load_order.pop('base')
    environment = isolated_environment()

    wait_until_quiet(arguments.max_load, arguments.patience)
    results = {
        'schema': 1,
        'kind': 'showcase',
        'started': datetime.datetime.now(datetime.UTC).isoformat(timespec='seconds'),
        'jess': jess_context(jess, arguments.runtime),
        'machine': machine_context(),
        'load_start': os.getloadavg(),
        'busiest_at_start': busiest_processes(),
        'runs': arguments.runs,
        'warmup': arguments.warmup,
        'load_order': load_order,
        'natives': {name: {'binary': tool.name, 'sha256': sha256(tool)} for name, tool in tools.items()},
        'workloads': [],
    }
    for workload in selected:
        print(f'bench: {workload.name}', flush=True)
        results['workloads'].append(
            run_workload(workload, jess, tools.get(workload.name), base, environment, arguments)
        )
    results['load_end'] = os.getloadavg()

    path = write_results(results)
    print_results(results)
    print(f'results: {path}')
    mismatches = [
        f'{workload["name"]} {output}'
        for workload in results['workloads']
        for output, verdict in workload['equivalence'].items()
        if verdict['status'] == 'mismatch'
    ]
    if mismatches:
        print(f"bench: outputs differ from the native tool's: {', '.join(mismatches)}", file=sys.stderr)
        return 1
    return 0


# Canaries: synthetic workloads for the Luau runtime and jess's bindings.


def canaries(arguments) -> int:
    jess = Path(arguments.jess).resolve()
    data = WORK / 'canaries' / 'data'
    scale = str(arguments.scale)
    subprocess.run(
        (str(jess), 'run', str(TOOLS / 'canaries' / 'setup.luau'), str(data), scale), check=True
    )
    environment = isolated_environment()
    selected = [name for name in CANARIES if not arguments.only or name in arguments.only]

    wait_until_quiet(arguments.max_load, arguments.patience)
    results = {
        'schema': 1,
        'kind': 'canaries',
        'started': datetime.datetime.now(datetime.UTC).isoformat(timespec='seconds'),
        'jess': jess_context(jess, arguments.runtime),
        'machine': machine_context(),
        'load_start': os.getloadavg(),
        'busiest_at_start': busiest_processes(),
        'runs': arguments.runs,
        'warmup': arguments.warmup,
        'scale': arguments.scale,
        'workloads': [],
    }
    for name in selected:
        command = [str(jess), 'run', str(TOOLS / 'canaries' / f'{name}.luau'), str(data), scale]
        log = WORK / 'canaries' / f'{name}.log'
        samples = []
        for round in range(arguments.warmup + arguments.runs):
            sample = measure(command, WORK, environment, log)
            # The canary's own timing of its hot loop, without process start and setup.
            sample['inner'] = float(re.findall(r'^canary (\S+)$', log.read_text(), re.MULTILINE)[-1])
            if round >= arguments.warmup:
                samples.append(sample)
        implementation = {
            'title': 'jess',
            'command': ['{tool}', 'run', f'tools/bench/canaries/{name}.luau', '{data}', scale],
            'samples': samples,
            'summary': summary(samples),
        }
        implementation['summary']['inner'] = summarize(samples, 'inner')
        script = TOOLS / 'canaries' / f'{name}.luau'
        results['workloads'].append(
            {'name': name, 'title': name, 'script_sha256': sha256(script), 'implementations': {'jess': implementation}}
        )
        print(f'  {name}: {implementation["summary"]["inner"]["median"]:.4f} s', flush=True)
    results['load_end'] = os.getloadavg()

    path = write_results(results)
    print_results(results)
    print(f'results: {path}')
    if not arguments.check:
        return 0
    return check_canaries(json.loads(Path(arguments.check).read_text()), results, arguments.tolerance)


def check_canaries(baseline: dict, results: dict, tolerance: float) -> int:
    if baseline['machine']['cpu'] != results['machine']['cpu'] or baseline.get('scale') != results['scale']:
        print('bench: the baseline is from another CPU or scale; its times say little about these', file=sys.stderr)
    before = {workload['name']: workload for workload in baseline['workloads']}
    slower = []
    for workload in results['workloads']:
        if workload['name'] not in before:
            continue
        old = before[workload['name']]['implementations']['jess']['summary']['inner']['median']
        new = workload['implementations']['jess']['summary']['inner']['median']
        if new > old * tolerance:
            slower.append(f'{workload["name"]} {new / old:.2f}x')
    if slower:
        print(
            f'bench: canaries slower than the baseline by more than {tolerance}x: {", ".join(slower)}', file=sys.stderr
        )
        return 1
    return 0


# Reports.


def seconds(value: float) -> str:
    return f'{value:.2f} s' if value < 10 else f'{value:.1f} s'


def size(value: float) -> str:
    mebibytes = value / (1 << 20)
    return f'{mebibytes:.0f} MiB' if mebibytes < 1024 else f'{mebibytes / 1024:.2f} GiB'


def print_results(results: dict) -> None:
    key = 'inner' if results['kind'] == 'canaries' else 'wall'
    print(f'\n{"workload":<18} {"":<7} {key + " median":>12} {"min–max":>17} {"cpu":>10} {"peak memory":>12}')
    for workload in results['workloads']:
        for implementation, measured in workload['implementations'].items():
            time_, cpu, rss = measured['summary'][key], measured['summary']['cpu'], measured['summary']['rss']
            spread = f'{time_["min"]:.3f}–{time_["max"]:.3f}'
            print(
                f'{workload["name"]:<18} {implementation:<7} {time_["median"]:>10.3f} s {spread:>17} '
                f'{seconds(cpu["median"]):>10} {size(rss["median"]):>12}'
            )
        for output, verdict in workload.get('equivalence', {}).items():
            details = f': {verdict["details"]}' if 'details' in verdict else ''
            print(f'{"":<18} {Path(output).name}: {verdict["status"]}{details}')
    print(f'load average {results["load_start"][0]:.2f} at the start, {results["load_end"][0]:.2f} at the end')


def compare(arguments) -> int:
    baseline, candidate = (json.loads(Path(path).read_text()) for path in (arguments.baseline, arguments.candidate))
    if baseline['kind'] != candidate['kind']:
        fail(f"can't compare {baseline['kind']} results with {candidate['kind']} results")

    for label, results in (('baseline', baseline), ('candidate', candidate)):
        jess, machine = results['jess'], results['machine']
        print(
            f'{label:<10} {jess["commit"][:10]}{"+dirty" if jess["dirty"] else ""} {jess["runtime"]}, {jess["luau"]}, '
            f'{jess["profile"]}; {machine["cpu"]} ({machine["threads"]} threads), load {results["load_start"][0]:.2f}; '
            f'{results["runs"]} runs'
        )
    for field in ('cpu', 'threads', 'memory_bytes', 'governor'):
        if baseline['machine'][field] != candidate['machine'][field]:
            print(f'warning: different {field}: {baseline["machine"][field]} and {candidate["machine"][field]}')
    if baseline.get('load_order', {}).get('config_sha256') != candidate.get('load_order', {}).get('config_sha256'):
        print('warning: different load orders')
    if baseline.get('scale') != candidate.get('scale'):
        print('warning: different canary scales')
    scripts = {workload['name']: workload.get('script_sha256') for workload in baseline['workloads']}
    for workload in candidate['workloads']:
        if workload.get('script_sha256') != scripts.get(workload['name'], workload.get('script_sha256')):
            print(f'warning: the {workload["name"]} canary changed')

    key = 'inner' if candidate['kind'] == 'canaries' else 'wall'
    before = {
        (workload['name'], implementation): measured
        for workload in baseline['workloads']
        for implementation, measured in workload['implementations'].items()
    }
    print(f'\n{"workload":<18} {"":<7} {"baseline":>10} {"candidate":>10} {"ratio":>7}', end=' ')
    print(f'{"memory":>10} {"memory":>10} {"ratio":>7}')
    for workload in candidate['workloads']:
        for implementation, measured in workload['implementations'].items():
            old = before.get((workload['name'], implementation))
            if not old:
                continue
            old_time, new_time = old['summary'][key]['median'], measured['summary'][key]['median']
            old_rss, new_rss = old['summary']['rss']['median'], measured['summary']['rss']['median']
            print(
                f'{workload["name"]:<18} {implementation:<7} {old_time:>8.3f} s {new_time:>8.3f} s',
                f'{new_time / old_time:>6.2f}x {size(old_rss):>10} {size(new_rss):>10} {new_rss / old_rss:>6.2f}x',
            )
    return 0


# Docs: the numbers the site prints, from one showcase results file.


def docs(arguments) -> int:
    path = Path(arguments.results).resolve()
    results = json.loads(path.read_text())
    if results['kind'] != 'showcase':
        fail('docs takes showcase results')
    if results['jess']['dirty'] or results['jess']['profile'] != 'release':
        fail('docs takes results from a release build of a clean checkout')

    machine, load_order = results['machine'], results['load_order']
    values = {
        'results': str(path.relative_to(ROOT)) if path.is_relative_to(ROOT) else path.name,
        'date': results['started'][:10],
        'commit': results['jess']['commit'][:7],
        'runtime': results['jess']['runtime'],
        'luau': results['jess']['luau'],
        'cpu': machine['cpu'],
        'threads': machine['threads'],
        'os': machine['os'],
        'load': f'{results["load_start"][0]:.2f}',
        'runs': results['runs'],
        'content_files': load_order['content_files'],
        'plugins': load_order['plugins'],
        'data_dirs': load_order['data_dirs'],
        'plugin_size': f'{load_order["plugin_bytes"] / 1e6:.0f} MB',
    }
    tables = {}
    for workload in results['workloads']:
        table = {}
        for implementation, measured in workload['implementations'].items():
            table[f'{implementation}_time'] = seconds(measured['summary']['wall']['median'])
            table[f'{implementation}_cpu'] = seconds(measured['summary']['cpu']['median'])
            table[f'{implementation}_memory'] = size(measured['summary']['rss']['median'])
        if 'native' in workload['implementations']:
            jess, native = (workload['implementations'][name]['summary'] for name in ('jess', 'native'))
            table['faster'] = f'{native["wall"]["median"] / jess["wall"]["median"]:.1f}'
            table['less_memory'] = f'{native["rss"]["median"] / jess["rss"]["median"]:.1f}'
        for name, value in workload['facts'].items():
            table[name.split('.', 1)[1].replace('.', '_')] = f'{value:,}'
        for output in workload['implementations']['jess']['outputs'].values():
            for tag, count in output.get('records', {}).items():
                table[f'written_{tag}'] = f'{count:,}'
        tables[workload['name']] = table

    lines = [
        "# Generated by `python3 tools/bench/bench.py docs` from the results file below; don't edit it.",
        '# Pages print these with the `bench` shortcode.',
    ]
    lines += [f'{key} = {json.dumps(value)}' for key, value in values.items()]
    for name, table in tables.items():
        lines.append(f'\n["{name}"]')
        lines += [f'{key} = {json.dumps(value)}' for key, value in table.items()]
    SITE_DATA.parent.mkdir(parents=True, exist_ok=True)
    SITE_DATA.write_text('\n'.join(lines) + '\n')
    print(f'wrote {SITE_DATA.relative_to(ROOT)}')
    return 0


def graph(arguments) -> int:
    # graph.py sits beside this file; importing it must not leave __pycache__ in the repo.
    sys.dont_write_bytecode = True
    sys.path.insert(0, str(TOOLS))
    import graph as drawing

    results = json.loads(Path(arguments.results).read_text())
    output = Path(arguments.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    drawing.draw(results, output)
    print(output)
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    commands = parser.add_subparsers(dest='command', required=True)

    def measured(name: str, help: str, names: list[str]) -> argparse.ArgumentParser:
        command = commands.add_parser(name, help=help)
        command.add_argument('--jess', default=ROOT / 'target' / 'release' / 'jess', help='the jess binary to measure')
        command.add_argument('--runs', type=int, default=5, help='measured runs of each (default 5)')
        command.add_argument('--warmup', type=int, default=1, help='unmeasured runs first (default 1)')
        command.add_argument('--only', nargs='+', choices=names, metavar='NAME', help=f'run only these: {names}')
        command.add_argument('--runtime', help='the Luau runtime jess is built on (default: l3i, from Cargo.lock)')
        command.add_argument(
            '--max-load', type=float, help='wait for the 1-minute load to drop below this first (default: no wait)'
        )
        command.add_argument('--patience', type=float, default=30, help='minutes to wait for it (default 30)')
        return command

    command = measured(
        'showcase',
        "Dreamwind's load-order tools and the native tools, over the real load order",
        [workload.name for workload in WORKLOADS],
    )
    command.add_argument('--openmw-cfg', help='the OpenMW config to read (default: the one OpenMW finds)')
    command.add_argument('--no-native', action='store_true', help="don't run the native tools")
    command.add_argument('--tes3merge', default=shutil.which('tes3merge'), help='native TES3Merge (default: on PATH)')
    command.add_argument(
        '--s3lightfixes', default=shutil.which('s3lightfixes'), help='native S3LightFixes (default: on PATH)'
    )
    command.add_argument('--merged-lands', help=f'native Merged Lands (default: build {MERGED_LANDS_COMMIT[:7]})')
    command.set_defaults(run=showcase)

    command = measured('canaries', 'synthetic runtime and binding workloads; no game data', CANARIES)
    command.add_argument('--scale', type=int, default=10, help='work per canary (default 10)')
    command.add_argument('--check', metavar='BASELINE', help='fail when a canary is slower than in these results')
    command.add_argument('--tolerance', type=float, default=1.5, help='how much slower --check allows (default 1.5)')
    command.set_defaults(run=canaries)

    command = commands.add_parser('compare', help='two results files side by side')
    command.add_argument('baseline')
    command.add_argument('candidate')
    command.set_defaults(run=compare)

    command = commands.add_parser('docs', help=f'write {SITE_DATA.relative_to(ROOT)} from showcase results')
    command.add_argument('results')
    command.set_defaults(run=docs)

    command = commands.add_parser('graph', help='draw showcase results')
    command.add_argument('results')
    command.add_argument('--output', default=WORK / 'showcase.png', help='the PNG to write (an SVG goes beside it)')
    command.set_defaults(run=graph)

    arguments = parser.parse_args()
    return arguments.run(arguments)


if __name__ == '__main__':
    sys.exit(main())
