"""Small subprocess and output helpers shared only by host tools."""
from __future__ import annotations
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]

def digest(path: Path) -> str:
    h = hashlib.sha256()
    with path.open('rb') as f:
        for block in iter(lambda: f.read(1 << 20), b''):
            h.update(block)
    return h.hexdigest()

def environment() -> dict[str, str]:
    env = os.environ.copy()
    # Some bundled toolchains name rustdoc differently; never install anything.
    if not env.get('RUSTDOC') and not shutil.which('rustdoc'):
        compiler = shutil.which('rustc')
        if compiler:
            candidate = Path(compiler).parent / 'rustdoc_tool_binary'
            if candidate.is_file():
                env['RUSTDOC'] = str(candidate)
    return env

def binary(name: str) -> str:
    value = shutil.which(name)
    if not value:
        raise RuntimeError(f'{name} is required on PATH; see docs/BUILD.md')
    return value

def create_directory(path: Path) -> Path:
    path = path.resolve()
    path.parent.mkdir(parents=True, exist_ok=True)
    path.mkdir()  # Refuse to mix current results with an existing run.
    return path

def write_json(path: Path, value: object) -> None:
    with path.open('x', encoding='utf-8') as f:
        json.dump(value, f, indent=2)
        f.write('\n')

def run(command: list[str], directory: Path, name: str,
        env: dict[str, str] | None = None, timeout: int = 240) -> dict:
    print('+ ' + ' '.join(str(x) for x in command), flush=True)
    started = time.monotonic()
    result = subprocess.run(command, cwd=ROOT, env=env or environment(),
                            text=True, stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, timeout=timeout)
    text = result.stdout
    with (directory / (name + '.log')).open('x', encoding='utf-8') as f:
        f.write(text)
    print(text, end='' if text.endswith('\n') else '\n', flush=True)
    record = {'name': name, 'command': command, 'returncode': result.returncode,
              'wall_seconds': time.monotonic() - started}
    if result.returncode:
        write_json(directory / (name + '-failure.json'), record)
        raise RuntimeError(f'{name} failed; see {directory / (name + ".log")}')
    return record

def versions() -> dict:
    result = {'python': sys.version}
    for name in ('rustc', 'cargo', 'rustfmt', 'clippy-driver'):
        exe = shutil.which(name)
        if exe:
            run = subprocess.run([exe, '--version'], capture_output=True,
                                 text=True, timeout=10, env=environment())
            result[name] = (run.stdout + run.stderr).strip()
        else:
            result[name] = 'not found'
    return result


def source_identity() -> dict:
    """A revision is evidence only when the corresponding worktree is recorded."""
    try:
        revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
        changes = subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT, text=True)
        return {'commit': revision, 'dirty': bool(changes), 'changes': changes.splitlines()}
    except (OSError, subprocess.SubprocessError):
        return {'commit': None, 'dirty': None}


def release_executable() -> Path:
    """Respect Cargo target_directory, including CARGO_TARGET_DIR/config overrides."""
    result = subprocess.check_output([binary('cargo'), 'metadata', '--format-version', '1',
                                      '--no-deps', '--locked', '--offline'],
                                     cwd=ROOT, env=environment(), text=True, timeout=30)
    target = Path(json.loads(result)['target_directory'])
    return target / 'release' / ('hachistep.exe' if os.name == 'nt' else 'hachistep')
