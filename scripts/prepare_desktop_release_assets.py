#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import shutil
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def read_cargo_package() -> dict[str, object]:
    return tomllib.loads((ROOT / "rust" / "crates" / "memorph" / "Cargo.toml").read_text(encoding="utf-8"))["package"]


def load_desktop_targets() -> list[dict[str, object]]:
    return tomllib.loads((ROOT / "platforms.toml").read_text(encoding="utf-8"))["desktop_targets"]


def select_desktop_targets(platform_ids: list[str]) -> list[dict[str, object]]:
    targets = load_desktop_targets()
    if not platform_ids:
        return targets
    selected = [target for target in targets if str(target["platform_id"]) in platform_ids]
    if not selected:
        raise SystemExit(f"[error] No desktop targets matched platform filters: {platform_ids}")
    return selected


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


def find_single_bundle(
    target_root: Path,
    version: str,
    extension: str,
    markers: list[str],
) -> Path:
    matches: list[Path] = []
    for path in target_root.rglob(f"*{extension}"):
        if not path.is_file():
            continue
        filename = path.name
        if version not in filename:
            continue
        lowered = filename.lower()
        if markers and not any(marker.lower() in lowered for marker in markers):
            continue
        matches.append(path)

    if not matches:
        raise FileNotFoundError(
            f"No desktop bundle found under {target_root} for version={version} extension={extension}"
        )
    if len(matches) != 1:
        names = ", ".join(str(path.relative_to(target_root)) for path in matches)
        raise RuntimeError(
            f"Expected exactly one desktop bundle for version={version} extension={extension}, found: {names}"
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
    parser.add_argument(
        "--platform-id",
        action="append",
        default=[],
        help="limit processing to one or more desktop platform ids",
    )
    args = parser.parse_args()

    cargo_package = read_cargo_package()
    config = tomllib.loads((ROOT / "platforms.toml").read_text(encoding="utf-8"))
    meta = config["meta"]
    desktop_targets = select_desktop_targets(args.platform_id)
    package_name = str(meta.get("binary_name", cargo_package["name"]))
    version = str(cargo_package["version"])

    target_root = (ROOT / args.target_root).resolve()
    assets_dir = (ROOT / args.assets_dir).resolve()
    assets_dir.mkdir(parents=True, exist_ok=True)

    copied_assets: list[Path] = []
    for desktop_target in desktop_targets:
        source = find_single_bundle(
            target_root=target_root,
            version=version,
            extension=desktop_target["extension"],
            markers=[str(item) for item in desktop_target.get("filename_markers", [])],
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
