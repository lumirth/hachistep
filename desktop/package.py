#!/usr/bin/env python3
"""Package existing desktop and CLI release builds for the current platform."""
import argparse
from pathlib import Path
import platform
import plistlib
import shutil
import subprocess
import sys
import tomllib

ROOT = Path(__file__).resolve().parent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--cli", type=Path)
    args = parser.parse_args()
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    core_version = tomllib.loads((ROOT.parent / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    if version != core_version:
        parser.error("Core and desktop release versions differ")
    systems = {"darwin": "macos", "win32": "windows", "linux": "linux"}
    if sys.platform not in systems:
        parser.error("Desktop packaging supports macOS, Windows, and Linux")
    arch = platform.machine().lower()
    arch = {"aarch64": "arm64", "amd64": "x86_64"}.get(arch, arch)
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    stage = out / "hachistep"
    stage.mkdir()
    assets = ROOT.parent / "assets/icons"
    suffix = ".exe" if sys.platform == "win32" else ""
    binary = args.binary or ROOT / f"target/release/hs-desktop{suffix}"
    cli = args.cli or ROOT.parent / f"target/release/hachistep{suffix}"
    shutil.copy2(cli, stage / f"hachistep-cli{suffix}")
    shutil.copy2(ROOT.parent / "LICENSE", stage / "LICENSE")
    if sys.platform == "darwin":
        contents = stage / "hachistep.app/Contents"
        (contents / "MacOS").mkdir(parents=True)
        (contents / "Resources").mkdir()
        shutil.copy2(binary, contents / "MacOS/hachistep")
        shutil.copy2(assets / "macos/icon.icns", contents / "Resources/Hachistep.icns")
        shutil.copy2(ROOT.parent / "LICENSE", contents / "Resources/LICENSE")
        with (contents / "Info.plist").open("wb") as file:
            plistlib.dump({
                "CFBundleExecutable": "hachistep", "CFBundleName": "hachistep",
                "CFBundleDisplayName": "hachistep", "CFBundleIdentifier": "me.kukas.hachistep",
                "CFBundlePackageType": "APPL", "CFBundleIconFile": "Hachistep.icns",
                "CFBundleShortVersionString": version.split("-")[0],
                "CFBundleVersion": version.split("-")[0],
                "NSHighResolutionCapable": True,
                "CFBundleDocumentTypes": [{
                    "CFBundleTypeName": "Pokéwalker image",
                    "CFBundleTypeRole": "Editor",
                    "LSHandlerRank": "Alternate",
                    "CFBundleTypeExtensions": ["bin", "rom", "eeprom"],
                }],
            }, file)
        subprocess.run(["codesign", "--force", "--sign", "-", "--timestamp=none", str(contents.parent)], check=True)
    else:
        shutil.copy2(binary, stage / f"hachistep{suffix}")
        if sys.platform == "linux":
            shutil.copy2(ROOT.parent / "assets/glyph.svg", stage / "hachistep.svg")
    archive = out / f"hachistep-{version}-{systems[sys.platform]}-{arch}"
    archive = shutil.make_archive(str(archive), "gztar" if sys.platform == "linux" else "zip", out, stage.name)
    print(archive)


if __name__ == "__main__":
    main()
