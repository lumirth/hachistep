"""Native and WASI command invocation for hachistep host tools."""
import shutil
from pathlib import Path


def binary(name: str) -> str:
    value = shutil.which(name)
    if not value:
        raise RuntimeError(f"{name} is required on PATH; see docs/build.md")
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
