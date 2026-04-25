import os
import platform
import sys
from importlib.resources import files


def main() -> None:
    system = platform.system()
    machine = platform.machine()

    mapping = {
        ("Darwin", "arm64"): "memorph_bin_darwin_arm64",
        ("Darwin", "x86_64"): "memorph_bin_darwin_x64",
        ("Linux", "x86_64"): "memorph_bin_linux_x64_gnu",
        ("Windows", "AMD64"): "memorph_bin_win32_x64_msvc",
    }

    module_name = mapping.get((system, machine))
    if module_name is None:
        print(f"Unsupported platform: {system} {machine}", file=sys.stderr)
        sys.exit(1)

    try:
        binary_name = "memorph.exe" if system == "Windows" else "memorph"
        binary_path = files(module_name) / "bin" / binary_name
    except Exception as e:
        print(f"Failed to locate memorph binary: {e}", file=sys.stderr)
        sys.exit(1)

    # execv replaces the current process, avoiding extra Python overhead
    os.execv(str(binary_path), [str(binary_path)] + sys.argv[1:])


if __name__ == "__main__":
    main()
