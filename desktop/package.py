#!/usr/bin/env python3
"""Package an existing release build for the current desktop platform."""
import argparse
from pathlib import Path
import plistlib
import shutil
import sys

ROOT = Path(__file__).resolve().parent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--binary", type=Path)
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    assets = ROOT.parent / "assets/icons"
    if sys.platform == "darwin":
        contents = out / "hachistep.app/Contents"
        (contents / "MacOS").mkdir(parents=True)
        (contents / "Resources").mkdir()
        shutil.copy2(args.binary or ROOT / "target/release/hs-desktop", contents / "MacOS/hachistep")
        shutil.copy2(assets / "macOS/Hachistep.icns", contents / "Resources/Hachistep.icns")
        shutil.copy2(ROOT.parent / "LICENSE", contents / "Resources/LICENSE")
        with (contents / "Info.plist").open("wb") as file:
            plistlib.dump({
                "CFBundleExecutable": "hachistep", "CFBundleName": "hachistep",
                "CFBundleDisplayName": "hachistep", "CFBundleIdentifier": "me.kukas.hachistep",
                "CFBundlePackageType": "APPL", "CFBundleIconFile": "Hachistep.icns",
                "CFBundleShortVersionString": "0.2.0", "CFBundleVersion": "1",
                "NSHighResolutionCapable": True,
            }, file)
        shutil.make_archive(str(out / "hachistep-macos"), "zip", out, "hachistep.app")
    elif sys.platform == "win32":
        shutil.copy2(args.binary or ROOT / "target/release/hs-desktop.exe", out / "hachistep.exe")
        shutil.copy2(ROOT.parent / "LICENSE", out / "LICENSE")
    else:
        parser.error("Desktop packaging supports macOS and Windows")
    print(out)


if __name__ == "__main__":
    main()
