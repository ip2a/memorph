#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import shutil
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

DESKTOP_TARGETS = [
    {
        "platform_id": "darwin-arm64",
        "target": "aarch64-apple-darwin",
        "extension": ".dmg",
    }
]


def read_cargo_package() -> dict[str, object]:
    return tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["package"]


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as file:
        for chunk in iter(lambda: file.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def display_path(path: Path) -> Path:
    try:
        return path.relative_to(ROOT)
    except ValueError:
        return path


def find_single_bundle(bundle_dirs: list[Path], extension: str) -> Path:
    for bundle_dir in bundle_dirs:
        matches = sorted(path for path in bundle_dir.glob(f"*{extension}") if path.is_file())
        if not matches:
            continue
        if len(matches) != 1:
            names = ", ".join(path.name for path in matches)
            raise RuntimeError(f"Expected exactly one desktop bundle in {bundle_dir}, found: {names}")
        return matches[0]
    searched = ", ".join(str(path) for path in bundle_dirs)
    raise FileNotFoundError(f"No desktop bundle found in: {searched}")


def main() -> None:
    parser = argparse.ArgumentParser(description="Prepare desktop release assets from Tauri bundle outputs")
    parser.add_argument(
        "--target-root",
        default="desktop/tauri/target",
        help="desktop target root containing Tauri bundle outputs",
    )
    parser.add_argument(
        "--assets-dir",
        default="desktop-release-assets",
        help="output directory for release-ready desktop assets",
    )
    args = parser.parse_args()

    cargo_package = read_cargo_package()
    package_name = str(cargo_package["name"])
    version = str(cargo_package["version"])

    target_root = (ROOT / args.target_root).resolve()
    assets_dir = (ROOT / args.assets_dir).resolve()
    assets_dir.mkdir(parents=True, exist_ok=True)

    copied_assets: list[Path] = []
    for desktop_target in DESKTOP_TARGETS:
        bundle_dirs = [
            target_root / desktop_target["target"] / "release" / "bundle" / "dmg",
            target_root / "release" / "bundle" / "dmg",
        ]
        source = find_single_bundle(bundle_dirs, desktop_target["extension"])
        destination = assets_dir / (
            f"{package_name}-desktop-v{version}-{desktop_target['platform_id']}{desktop_target['extension']}"
        )
        shutil.copy2(source, destination)
        copied_assets.append(destination)
        print(f"[ok] wrote {display_path(destination)}")

    checksum_path = assets_dir / "SHA256SUMS-desktop.txt"
    checksum_path.write_text(
        "".join(f"{sha256(path)}  {path.name}\n" for path in copied_assets),
        encoding="utf-8",
    )
    print(f"[ok] wrote {display_path(checksum_path)}")


if __name__ == "__main__":
    main()
