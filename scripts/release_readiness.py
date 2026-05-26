#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import subprocess
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def read_cargo_package() -> dict[str, object]:
    cargo_toml = (ROOT / "rust" / "crates" / "memorph" / "Cargo.toml").read_text(encoding="utf-8")
    return tomllib.loads(cargo_toml)["package"]


def load_platform_config() -> tuple[dict[str, str], list[dict[str, str]], list[dict[str, object]]]:
    data = tomllib.loads((ROOT / "platforms.toml").read_text(encoding="utf-8"))
    return data["meta"], data["platforms"], data.get("desktop_targets", [])


def run_check(command: list[str]) -> None:
    subprocess.run(command, cwd=ROOT, check=True)


def build_readiness_report() -> dict[str, object]:
    cargo_package = read_cargo_package()
    meta, platforms, desktop_targets = load_platform_config()
    cargo_name = str(cargo_package["name"])
    binary_name = str(meta["binary_name"])
    version = str(cargo_package["version"])
    repository = str(cargo_package["repository"])

    npm_packages = [binary_name, *[platform["npm_package"] for platform in platforms]]
    python_packages = [binary_name, *[platform["python_package"] for platform in platforms]]
    platform_ids = [platform["id"] for platform in platforms]
    desktop_assets = [
        {
            "platform_id": str(target["platform_id"]),
            "bundle": str(target["bundle"]),
            "extension": str(target["extension"]),
        }
        for target in desktop_targets
    ]

    return {
        "package_name": binary_name,
        "version": version,
        "repository": repository,
        "binary_name": binary_name,
        "platforms": platform_ids,
        "desktop_assets": desktop_assets,
        "npm_packages": npm_packages,
        "python_packages": python_packages,
        "workflows": {
            "build": ".github/workflows/release-build.yml",
            "build_desktop": ".github/workflows/release-build-desktop.yml",
            "update_latest_json": ".github/workflows/release-update-latest-json.yml",
            "publish_npm": ".github/workflows/release-publish-npm.yml",
            "publish_pypi": ".github/workflows/release-publish-pypi.yml",
            "publish_crates": ".github/workflows/release-publish-crates.yml",
            "post_release_verify": ".github/workflows/post-release-verify.yml",
        },
        "official_install": {
            "shell": "curl -fsSL https://raw.githubusercontent.com/ip2a/memorph/main/install.sh | bash",
            "direct_release": f"{repository}/releases",
        },
        "environments": {
            "npm": "npm",
            "pypi": "pypi",
            "crates": "crates",
        },
        "trusted_publishers": {
            "npm": {
                "repository": repository.removeprefix("https://github.com/"),
                "workflow": ".github/workflows/release-publish-npm.yml",
                "environment": "npm",
                "packages": npm_packages,
            },
            "pypi": {
                "repository": repository.removeprefix("https://github.com/"),
                "workflow": ".github/workflows/release-publish-pypi.yml",
                "environment": "pypi",
                "packages": python_packages,
            },
            "crates": {
                "repository": repository.removeprefix("https://github.com/"),
                "workflow": ".github/workflows/release-publish-crates.yml",
                "environment": "crates",
                "packages": [cargo_name, binary_name],
            },
        },
        "local_checks": [
            "python3 scripts/release_preflight.py",
            "python3 scripts/test_release_scripts.py",
            "python3 scripts/test_web_ui_invariants.py",
        ],
        "release_sequence": [
            "Update Cargo.toml version",
            "uv run python scripts/sync_version.py",
            "python3 scripts/release_preflight.py",
            "python3 scripts/test_release_scripts.py",
            "Commit version changes",
            "Push tag vX.Y.Z",
            "Wait for release-build and release-build-desktop to finish",
            "Record build_run_id from release-build",
            "Run release-publish-crates on the release tag",
            "Run release-publish-npm with build_run_id",
            "Run release-publish-pypi with build_run_id",
            "Run post-release-verify with version X.Y.Z",
            "Run release-update-latest-json with version X.Y.Z",
            "Confirm install.sh installs the released version from GitHub Release",
            "Confirm desktop release assets exist on the same GitHub Release",
        ],
    }


def print_text_report(report: dict[str, object]) -> None:
    print(f"Package: {report['package_name']} v{report['version']}")
    print(f"Repository: {report['repository']}")
    print(f"Binary: {report['binary_name']}")
    print("Platforms:")
    for platform in report["platforms"]:
        print(f"  - {platform}")
    print("Desktop Assets:")
    for item in report["desktop_assets"]:
        print(f"  - {item['platform_id']} ({item['bundle']}{item['extension']})")
    print("Official Install:")
    print(f"  - shell: {report['official_install']['shell']}")
    print(f"  - direct release: {report['official_install']['direct_release']}")
    print("GitHub Environments:")
    for name, value in report["environments"].items():
        print(f"  - {name}: {value}")
    print("Trusted Publishers:")
    trusted_publishers = report["trusted_publishers"]
    for registry, config in trusted_publishers.items():
        print(f"  - {registry}:")
        print(f"      repository: {config['repository']}")
        print(f"      workflow: {config['workflow']}")
        print(f"      environment: {config['environment']}")
        print(f"      packages: {', '.join(config['packages'])}")
    print("Local Checks:")
    for command in report["local_checks"]:
        print(f"  - {command}")
    print("Release Sequence:")
    for idx, step in enumerate(report["release_sequence"], start=1):
        print(f"  {idx}. {step}")


def main() -> None:
    parser = argparse.ArgumentParser(description="Show trusted release readiness for this repository")
    parser.add_argument("--json", action="store_true", help="output machine-readable JSON")
    parser.add_argument(
        "--check-local",
        action="store_true",
        help="run local release gates before printing the report",
    )
    args = parser.parse_args()

    if args.check_local:
        run_check(["python3", "scripts/release_preflight.py"])
        run_check(["python3", "scripts/test_release_scripts.py"])

    report = build_readiness_report()
    if args.json:
        print(json.dumps(report, ensure_ascii=False, indent=2))
        return
    print_text_report(report)


if __name__ == "__main__":
    main()
