"""Build embedded production assets without starting a filesystem watcher."""
import argparse
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parent.parent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args()
    command = [
        "cargo", "build", "--locked", "--release",
        "-p", "epub-reference-studio", "--features", "custom-protocol",
    ]
    if args.offline:
        command.append("--offline")
    subprocess.run(command, cwd=ROOT, check=True)
    print(ROOT / "target/release/epub-reference-studio")


if __name__ == "__main__":
    main()
