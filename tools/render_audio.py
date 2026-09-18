#!/usr/bin/env python3
"""Integrate timestamped piezo drive over sample intervals to produce a mono WAV.

Requires a complete trace and its report. See docs/INPUTS.md for the rendering model.
"""
from __future__ import annotations
import argparse
import json
from pathlib import Path
import re
import struct
import wave

ONE = 1 << 64
LEVELS = {'Negative': -1, 'Neutral': 0, 'Positive': 1}

def load(trace: Path, report: Path) -> tuple[int, list[tuple[int, int]]]:
    metadata = json.loads(report.read_text())
    if not metadata.get('trace_complete') or metadata.get('trace_dropped', 0):
        raise ValueError('report does not certify a complete selected trace')
    events = []
    count, previous = 0, 0
    end = int(metadata['time_raw'])
    with trace.open(encoding='utf-8') as f:
        for line in f:
            count += 1
            stamp, separator, body = line.rstrip('\n').partition('\t')
            if not separator or not re.fullmatch('[0-9a-fA-F]{32}', stamp):
                raise ValueError(f'bad trace line {count}')
            at = int(stamp, 16)
            if at < previous or at > end:
                raise ValueError('nonmonotonic or out-of-range trace')
            previous = at
            if body.startswith('Buzzer '):
                match = re.search(r'drive: (Negative|Neutral|Positive)\b', body)
                if not match:
                    raise ValueError(f'bad piezo event on line {count}')
                events.append((at, LEVELS[match.group(1)]))
    if count != metadata['trace_records']:
        raise ValueError('trace record count does not match report')
    if len(events) != metadata['buzzer_events']:
        raise ValueError('buzzer event count does not match report')
    return end, events

def samples(end: int, events: list[tuple[int, int]], rate: int, amplitude: int):
    if not 1000 <= rate <= 192000 or not 1 <= amplitude <= 32767:
        raise ValueError('rate or amplitude is outside the supported range')
    index, drive = 0, 0
    count = (end * rate + ONE - 1) // ONE
    for i in range(count):
        left = i * ONE // rate
        right = min((i + 1) * ONE // rate, end)
        while index < len(events) and events[index][0] <= left:
            drive = events[index][1]
            index += 1
        cursor, area = left, 0
        while index < len(events) and events[index][0] < right:
            at, level = events[index]
            area += drive * (at - cursor)
            cursor, drive = at, level
            index += 1
        area += drive * (right - cursor)
        width = right - left
        value = (area * amplitude + width // 2) // width if width else 0
        yield max(-32768, min(32767, value))

def main() -> None:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--trace', type=Path, required=True)
    p.add_argument('--report', type=Path, required=True)
    p.add_argument('--out', type=Path, required=True)
    p.add_argument('--rate', type=int, default=48000)
    p.add_argument('--amplitude', type=int, default=10000)
    a = p.parse_args()
    end, events = load(a.trace, a.report)
    if not 1000 <= a.rate <= 192000 or not 1 <= a.amplitude <= 32767:
        raise ValueError('invalid rate or amplitude')
    with a.out.open('xb') as raw, wave.open(raw, 'wb') as wav:
        wav.setnchannels(1)
        wav.setsampwidth(2)
        wav.setframerate(a.rate)
        block = bytearray()
        for sample in samples(end, events, a.rate, a.amplitude):
            block.extend(struct.pack('<h', sample))
            if len(block) >= 16384:
                wav.writeframesraw(block)
                block.clear()
        wav.writeframesraw(block)
    print(f'{len(events)} piezo transitions rendered; {a.out}')
if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, KeyError) as e:
        raise SystemExit(str(e))
