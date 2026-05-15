#!/usr/bin/env python3
from __future__ import annotations

import json
import re
import subprocess
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
NPM_REPOSITORY_URL = "https://github.com/ip2a/memorph"


def read_cargo_version() -> str:
    cargo_toml = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    match = re.search(r'^version\s*=\s*"([^"]+)"', cargo_toml, flags=re.MULTILINE)
    if not match:
        raise RuntimeError("Version not found in Cargo.toml")
    return match.group(1)


def update_json_version(path: Path, version: str) -> None:
    data = json.loads(path.read_text(encoding="utf-8"))
    data["version"] = version
    repository = data.get("repository")
    if not isinstance(repository, dict):
        repository = {"type": "git"}
    repository["type"] = "git"
    repository["url"] = NPM_REPOSITORY_URL
    data["repository"] = repository
    if "optionalDependencies" in data:
        for name in data["optionalDependencies"]:
            data["optionalDependencies"][name] = version
    path.write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def update_pyproject_version(path: Path, version: str) -> None:
    lines = path.read_text(encoding="utf-8").splitlines(keepends=True)
    in_project = False
    replaced = False
    for idx, line in enumerate(lines):
        stripped = line.strip()
        if stripped == "[project]":
            in_project = True
            continue
        if stripped.startswith("[") and stripped != "[project]":
            in_project = False
        if in_project and stripped.startswith("version ="):
            lines[idx] = f'version = "{version}"\n'
            replaced = True
    if not replaced:
        raise RuntimeError(f"Missing [project].version in file: {path}")
    path.write_text("".join(lines), encoding="utf-8")


def load_platforms() -> list[dict[str, str]]:
    platforms_toml = (ROOT / "platforms.toml").read_text(encoding="utf-8")
    return tomllib.loads(platforms_toml)["platforms"]


def update_memorph_python_dependencies(version: str) -> None:
    path = ROOT / "python" / "packages" / "memorph" / "pyproject.toml"
    lines = path.read_text(encoding="utf-8").splitlines(keepends=True)

    in_project = False
    dep_start = None
    dep_end = None
    for idx, line in enumerate(lines):
        stripped = line.strip()
        if stripped == "[project]":
            in_project = True
            continue
        if stripped.startswith("[") and stripped != "[project]":
            in_project = False
        if in_project and stripped.startswith("dependencies = ["):
            dep_start = idx
            continue
        if dep_start is not None and dep_end is None and stripped == "]":
            dep_end = idx
            break

    if dep_start is None or dep_end is None:
        raise RuntimeError(f"Could not find [project].dependencies block in: {path}")

    dep_indent = "  "
    new_deps = []
    for platform in load_platforms():
        package = platform["python_package"]
        marker = platform["python_marker"]
        new_deps.append(f'{dep_indent}"{package}=={version}; {marker}",\n')

    lines[dep_start + 1 : dep_end] = new_deps
    path.write_text("".join(lines), encoding="utf-8")
    print(f"[ok] Synced main package dependency versions: {path.relative_to(ROOT)}")


def sync_cargo_lockfile() -> None:
    subprocess.run(["cargo", "generate-lockfile"], cwd=ROOT, check=True)
    print("[ok] Synced Cargo.lock")


def main() -> None:
    version = read_cargo_version()
    print(f"[info] Unified version: {version}")

    npm_packages_dir = ROOT / "npm" / "packages"
    if not npm_packages_dir.exists():
        raise RuntimeError(f"Missing npm packages directory: {npm_packages_dir}")
    json_files = sorted(npm_packages_dir.rglob("package.json"))
    if not json_files:
        raise RuntimeError(f"No npm package.json files found under: {npm_packages_dir}")
    for json_file in json_files:
        update_json_version(json_file, version)
        print(f"[ok] Synced version: {json_file.relative_to(ROOT)}")

    pyproject_files = sorted((ROOT / "python" / "packages").rglob("pyproject.toml"))
    for pyproject_file in pyproject_files:
        update_pyproject_version(pyproject_file, version)
        print(f"[ok] Synced version: {pyproject_file.relative_to(ROOT)}")

    update_memorph_python_dependencies(version)
    sync_cargo_lockfile()


if __name__ == "__main__":
    main()
