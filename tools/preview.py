#!/usr/bin/env python3
"""Make a standalone HTML viewer for captured PGM frames."""
from __future__ import annotations
import argparse
import base64
import html
from pathlib import Path
import re

def read_pgm(path: Path) -> tuple[int, int, bytes]:
    data = path.read_bytes()
    match = re.match(rb'P5\s+(\d+)\s+(\d+)\s+255(?:\r\n|[\n\r \t])', data)
    if not match:
        raise ValueError(f'{path}: expected binary PGM with max value 255')
    width, height = map(int, match.groups())
    pixels = data[match.end():]
    if not 0 < width <= 4096 or not 0 < height <= 4096 or len(pixels) != width * height:
        raise ValueError(f'{path}: invalid dimensions or pixel count')
    return width, height, pixels

def render(frames: list[Path], out: Path) -> None:
    pieces = ['<!doctype html><meta charset="utf-8"><title>HachiStep captured frames</title>',
              '<style>body{font:18px system-ui;max-width:900px;margin:3rem auto;padding:1rem}canvas{image-rendering:pixelated;max-width:100%;border:1px solid}figure{margin:2rem 0}</style>',
              '<h1>Captured firmware output</h1><p>These are emulator LCD captures, not hardware photographs or an interactive frontend.</p>']
    for i, frame in enumerate(frames):
        w, h, pixels = read_pgm(frame)
        title = f'{frame.parent.name}/{frame.name}'
        encoded = base64.b64encode(pixels).decode('ascii')
        pieces.append(f'<figure><canvas id="c{i}" width="{w}" height="{h}" style="width:{w*5}px"></canvas><figcaption>{html.escape(title)}</figcaption></figure>')
        pieces.append(f'<script>{{let c=document.getElementById("c{i}"),x=c.getContext("2d"),p=atob("{encoded}"),d=x.createImageData({w},{h});for(let j=0;j<p.length;j++){{let v=p.charCodeAt(j);d.data.set([v,v,v,255],j*4)}}x.putImageData(d,0,0)}}</script>')
    with out.open('x', encoding='utf-8') as f:
        f.write('\n'.join(pieces))

def main() -> None:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('frames', nargs='+', type=Path)
    p.add_argument('--out', required=True, type=Path)
    a = p.parse_args()
    render(a.frames, a.out)
if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError) as e:
        raise SystemExit(str(e))
