"""Small subprocess and output helpers shared only by host tools."""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def digest(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def environment() -> dict[str, str]:
    env = os.environ.copy()
    # Some bundled toolchains name rustdoc differently; never install anything.
    if not env.get("RUSTDOC") and not shutil.which("rustdoc"):
        compiler = shutil.which("rustc")
        if compiler:
            candidate = Path(compiler).parent / "rustdoc_tool_binary"
            if candidate.is_file():
                env["RUSTDOC"] = str(candidate)
    return env


def binary(name: str) -> str:
    value = shutil.which(name)
    if not value:
        raise RuntimeError(f"{name} is required on PATH; see docs/BUILD.md")
    return value


def cli_command(executable: Path, arguments: list[str]) -> list[str]:
    """Run a native CLI or its WASI build with the same arguments."""
    command = [str(executable.resolve()), *arguments]
    if executable.suffix != ".wasm":
        return command
    # Callers pass absolute file paths and create the run's parent directory.
    # A trace inside the new output directory uses that parent's grant too.
    parents = {
        Path(arg).parent
        for arg in arguments
        if Path(arg).is_absolute() and Path(arg).parent.is_dir()
    }
    roots = sorted(p for p in parents if not any(q in p.parents for q in parents))
    return [
        binary("wasmtime"),
        "run",
        *(part for root in roots for part in ("--dir", str(root))),
        *command,
    ]


def create_directory(path: Path) -> Path:
    path = path.resolve()
    path.parent.mkdir(parents=True, exist_ok=True)
    path.mkdir()  # Refuse to mix current results with an existing run.
    return path


def write_json(path: Path, value: object) -> None:
    with path.open("x", encoding="utf-8") as f:
        json.dump(value, f, indent=2)
        f.write("\n")


def run(
    command: list[str],
    directory: Path,
    name: str,
    env: dict[str, str] | None = None,
    timeout: int = 240,
) -> dict:
    print("+ " + " ".join(str(x) for x in command), flush=True)
    started = time.monotonic()
    result = subprocess.run(
        command,
        cwd=ROOT,
        env=env or environment(),
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        timeout=timeout,
        check=False,
    )
    text = result.stdout
    with (directory / (name + ".log")).open("x", encoding="utf-8") as f:
        f.write(text)
    print(text, end="" if text.endswith("\n") else "\n", flush=True)
    record = {
        "name": name,
        "command": command,
        "returncode": result.returncode,
        "wall_seconds": time.monotonic() - started,
    }
    if result.returncode:
        write_json(directory / (name + "-failure.json"), record)
        raise RuntimeError(f"{name} failed; see {directory / (name + '.log')}")
    return record


def versions() -> dict:
    result = {"python": sys.version}
    for name in ("rustc", "cargo", "rustfmt", "clippy-driver", "wasmtime"):
        exe = shutil.which(name)
        if exe:
            run = subprocess.run(
                [exe, "--version"],
                capture_output=True,
                text=True,
                timeout=10,
                env=environment(),
                check=False,
            )
            result[name] = (run.stdout + run.stderr).strip()
        else:
            result[name] = "not found"
    return result


def source_identity(directory: Path = ROOT) -> dict:
    """Identify tracked and nonignored untracked content, including dirty files."""
    try:
        command = ["git", "--no-optional-locks", "-C", str(directory)]

        def git(*args: str) -> bytes:
            return subprocess.check_output([*command, *args], stderr=subprocess.DEVNULL)

        root = Path(os.fsdecode(git("rev-parse", "--show-toplevel")).strip())
        command = ["git", "--no-optional-locks", "-C", str(root)]
        revision = git("rev-parse", "HEAD").decode().strip()
        changes = git("status", "--porcelain", "--untracked-files=all").decode()
        names = git("ls-files", "--cached", "--others", "--exclude-standard", "-z")
        tree = hashlib.sha256()
        for name in sorted(set(names.split(b"\0")) - {b""}):
            path = root / os.fsdecode(name)
            if path.is_symlink():
                kind, value = (
                    "symlink",
                    hashlib.sha256(os.fsencode(os.readlink(path))).hexdigest(),
                )
            elif path.is_file():
                kind, value = (
                    "executable" if path.stat().st_mode & 0o111 else "file",
                    digest(path),
                )
            elif not path.exists():
                kind, value = "missing", ""
            else:
                raise OSError(f"cannot fingerprint {path}")
            tree.update(json.dumps([os.fsdecode(name), kind, value]).encode() + b"\n")
        return {
            "commit": revision,
            "dirty": bool(changes),
            "changes": changes.splitlines(),
            "tree_sha256": tree.hexdigest(),
        }
    except (OSError, subprocess.SubprocessError) as error:
        return {"commit": None, "dirty": None, "tree_sha256": None, "error": str(error)}


def write_summary(path: Path, value: dict, source_before: dict) -> None:
    """Retain both source identities and reject a run whose checkout changed."""
    after = source_identity()
    unchanged = (
        source_before["tree_sha256"] == after["tree_sha256"]
        if source_before["tree_sha256"] and after["tree_sha256"]
        else None
    )
    write_json(
        path,
        {
            **value,
            "source": source_before,
            "source_after": after,
            "source_unchanged": unchanged,
        },
    )
    if unchanged is False:
        raise RuntimeError(f"source changed during execution; see {path}")


def release_executable() -> Path:
    """Respect Cargo target_directory, including CARGO_TARGET_DIR/config overrides."""
    result = subprocess.check_output(
        [
            binary("cargo"),
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--locked",
            "--offline",
        ],
        cwd=ROOT,
        env=environment(),
        text=True,
        timeout=30,
    )
    target = Path(json.loads(result)["target_directory"])
    return target / "release" / ("hachistep.exe" if os.name == "nt" else "hachistep")
