#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import re
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


def find_single_bundle(target_root: Path, version: str, platform_id: str, extension: str) -> Path:
    architecture_markers = {
        "darwin-arm64": ["aarch64", "arm64"],
    }
    version_pattern = re.escape(version)
    matches: list[Path] = []
    for path in target_root.rglob(f"*{extension}"):
        if not path.is_file():
            continue
        if "bundle" not in path.parts:
            continue
        filename = path.name
        if not re.search(version_pattern, filename):
            continue
        if not any(marker in filename for marker in architecture_markers.get(platform_id, [])):
            continue
        matches.append(path)

    if not matches:
        raise FileNotFoundError(
            f"No desktop bundle found under {target_root} for version={version} platform={platform_id}"
        )
    if len(matches) != 1:
        names = ", ".join(str(path.relative_to(target_root)) for path in matches)
        raise RuntimeError(
            f"Expected exactly one desktop bundle for version={version} platform={platform_id}, found: {names}"
        )
    return matches[0]


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
        source = find_single_bundle(
            target_root=target_root,
            version=version,
            platform_id=desktop_target["platform_id"],
            extension=desktop_target["extension"],
        )
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
