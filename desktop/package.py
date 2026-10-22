#!/usr/bin/env python3
"""Package the desktop release build for the current platform."""
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
    archive = out / f"hachistep-{version}-{systems[sys.platform]}-{arch}"
    if sys.platform == "darwin":
        import dmgbuild

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
        archive = f"{archive}.dmg"
        dmgbuild.build_dmg(str(archive), "hachistep", settings={
            "format": "UDZO",
            "files": [str(contents.parent)],
            "symlinks": {"Applications": "/Applications"},
            "icon": str(assets / "macos/icon.icns"),
            "background": str(ROOT.parent / "assets/dmg/background.png"),
            # Finder can keep the user's path and status bars visible.
            # Leave room below the icon labels.
            "window_rect": ((200, 200), (640, 420)),
            "icon_locations": {"hachistep.app": (166, 220), "Applications": (474, 220)},
            "icon_size": 112,
            "text_size": 14,
        })
    else:
        shutil.copy2(binary, stage / f"hachistep{suffix}")
        shutil.copy2(ROOT.parent / "LICENSE", stage / "LICENSE")
        if sys.platform == "linux":
            shutil.copy2(ROOT.parent / "assets/glyph.svg", stage / "hachistep.svg")
        archive = shutil.make_archive(str(archive), "gztar" if sys.platform == "linux" else "zip", out, stage.name)
    print(archive)


if __name__ == "__main__":
    main()
