"""Draws showcase results: wall time and peak memory, native against jess, as an SVG and a PNG.

Called by `bench.py graph`. The SVG is written by hand, so nothing needs installing; ImageMagick
(`magick`) turns it into the PNG.
"""

from __future__ import annotations

import subprocess
from pathlib import Path
from xml.sax.saxutils import escape

WIDTH, HEIGHT = 1600, 900
BACKGROUND, TEXT, DIM, FAINT, AXIS = '#0b0b0d', '#f0f0f2', '#8a8f99', '#555a63', '#1d1f24'
NATIVE, JESS = '#6a707c', '#b36bff'
SANS = "'Fira Sans', Cantarell, 'Adwaita Sans', sans-serif"
MONO = "'Fira Code', 'Fira Mono', monospace"

# Each panel: where its bars start, how wide the longest one is, and where its ratio goes.
PANELS = (
    ('WALL TIME', 'wall', 320, 350, 832, 'faster'),
    ('PEAK MEMORY', 'rss', 992, 336, 1515, 'less'),
)
TOP, BOTTOM, BAR, GAP = 230, 810, 50, 12


def seconds(value: float) -> str:
    return f'{value:.2f} s' if value < 10 else f'{value:.1f} s'


def size(value: float) -> str:
    mebibytes = value / (1 << 20)
    return f'{mebibytes:.0f} MiB' if mebibytes < 1024 else f'{mebibytes / 1024:.2f} GiB'


FORMATS = {'wall': seconds, 'rss': size}


def text(
    x: float, y: float, content: str, size: int, fill: str, family: str = MONO, anchor: str = 'start', weight: int = 400
) -> str:
    return (
        f'<text x="{x:.1f}" y="{y:.1f}" font-family="{family}" font-size="{size}" font-weight="{weight}" '
        f'fill="{fill}" text-anchor="{anchor}">{escape(content)}</text>'
    )


def draw(results: dict, output: Path) -> None:
    workloads = [workload for workload in results['workloads'] if 'native' in workload['implementations']]
    row = (BOTTOM - TOP) / len(workloads)
    parts = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{HEIGHT}" viewBox="0 0 {WIDTH} {HEIGHT}">',
        f'<rect width="{WIDTH}" height="{HEIGHT}" fill="{BACKGROUND}"/>',
        f'<text x="80" y="118" font-family="{SANS}" font-size="84" font-weight="600" fill="{TEXT}">jess'
        f'<tspan fill="{JESS}">_</tspan></text>',
    ]

    for index, (label, color) in enumerate((('native', NATIVE), ('jess · luau', JESS))):
        x = 1175 + index * 168
        parts.append(f'<rect x="{x}" y="88" width="22" height="22" fill="{color}"/>')
        parts.append(text(x + 30, 106, label, 20, DIM))

    for title, key, left, span, ratio_x, word in PANELS:
        parts.append(text(left, 188, title, 20, DIM, weight=700))
        parts.append(f'<line x1="{left}" y1="{TOP - 14}" x2="{left}" y2="{BOTTOM}" stroke="{AXIS}" stroke-width="2"/>')
        largest = max(
            workload['implementations'][name]['summary'][key]['max']
            for workload in workloads
            for name in ('native', 'jess')
        )
        for index, workload in enumerate(workloads):
            middle = TOP + row * (index + 0.5)
            for offset, name, color, label_color in ((-1, 'native', NATIVE, DIM), (1, 'jess', JESS, TEXT)):
                measured = workload['implementations'][name]['summary'][key]
                y = middle - BAR - GAP / 2 if offset < 0 else middle + GAP / 2
                scale = span / largest
                end = left + measured['median'] * scale
                low, high = left + measured['min'] * scale, left + measured['max'] * scale
                parts.append(
                    f'<rect x="{left}" y="{y:.1f}" width="{max(end - left, 2):.1f}" height="{BAR}" fill="{color}"/>'
                )
                parts.append(
                    f'<line x1="{low:.1f}" y1="{y + BAR / 2:.1f}" x2="{high:.1f}" y2="{y + BAR / 2:.1f}" '
                    f'stroke="{DIM}" stroke-width="1.5"/>'
                )
                parts.append(
                    text(max(end, high) + 8, y + BAR / 2 + 7, FORMATS[key](measured['median']), 19, label_color)
                )

            native = workload['implementations']['native']['summary'][key]['median']
            jess = workload['implementations']['jess']['summary'][key]['median']
            parts.append(text(ratio_x, middle - 4, f'{native / jess:.1f}×', 26, JESS, SANS, 'middle', 600))
            parts.append(text(ratio_x, middle + 26, word, 26, JESS, SANS, 'middle', 600))

    for index, workload in enumerate(workloads):
        lines = workload['title'].split('\n')
        middle = TOP + row * (index + 0.5)
        for number, line in enumerate(lines):
            y = middle + 9 + (number - (len(lines) - 1) / 2) * 30
            parts.append(text(300, y, line, 25, TEXT, SANS, 'end', 600))

    parts.append(text(1520, 868, f'median of {results["runs"]} · whiskers min–max', 17, FAINT, anchor='end'))
    parts.append('</svg>')

    svg = output.with_suffix('.svg')
    svg.write_text('\n'.join(parts) + '\n')
    subprocess.run(('magick', '-background', BACKGROUND, str(svg), str(output)), check=True)
