#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def read_cargo_package() -> dict[str, object]:
    return tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["package"]


def list_release_assets(directory: Path) -> list[dict[str, str]]:
    assets = []
    for path in sorted(directory.iterdir()):
        if not path.is_file():
            continue
        assets.append({"name": path.name})
    return assets


def display_path(path: Path) -> Path:
    try:
        return path.relative_to(ROOT)
    except ValueError:
        return path


def main() -> None:
    parser = argparse.ArgumentParser(description="Generate latest.json manifest for GitHub release assets")
    parser.add_argument("--version", help="release version; defaults to Cargo.toml version")
    parser.add_argument("--tag", help="release tag; defaults to v<version>")
    parser.add_argument("--base-url", required=True, help="GitHub release download base URL")
    parser.add_argument(
        "--assets-dir",
        action="append",
        default=[],
        help="asset directory to include; can be specified multiple times",
    )
    parser.add_argument("--output", default="latest.json", help="output manifest path")
    args = parser.parse_args()

    cargo_package = read_cargo_package()
    version = args.version or str(cargo_package["version"])
    tag = args.tag or f"v{version}"

    assets: list[dict[str, str]] = []
    for directory in args.assets_dir:
        path = (ROOT / directory).resolve()
        for asset in list_release_assets(path):
            asset["url"] = f"{args.base_url}/{asset['name']}"
            assets.append(asset)

    payload = {
        "name": str(cargo_package["name"]),
        "version": f"v{version}",
        "tag": tag,
        "release_url": f"https://github.com/{str(cargo_package['repository']).removeprefix('https://github.com/')}/releases/tag/{tag}",
        "assets": assets,
    }
    output_path = (ROOT / args.output).resolve()
    output_path.write_text(json.dumps(payload, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"[ok] wrote {display_path(output_path)}")


if __name__ == "__main__":
    main()
