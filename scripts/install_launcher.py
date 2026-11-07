"""Install the built application and launcher for the current desktop user."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent


def atomic_copy(source, destination, mode):
    destination.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(dir=destination.parent)
    os.close(fd)
    try:
        shutil.copyfile(source, temporary)
        os.chmod(temporary, mode)
        os.replace(temporary, destination)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def main():
    source = ROOT / "target/release/epub-reference-studio"
    if not source.is_file():
        raise SystemExit("Run python3 scripts/build.py first.")
    data = Path(os.environ.get(
        "XDG_DATA_HOME", Path.home() / ".local/share",
    ))
    binary = Path.home() / ".local/lib/epub-reference-studio" / source.name
    atomic_copy(source, binary, 0o755)
    icon = data / "icons/hicolor/128x128/apps/epub-reference-studio.png"
    atomic_copy(ROOT / "src-tauri/icons/128x128.png", icon, 0o644)
    applications = data / "applications"
    applications.mkdir(parents=True, exist_ok=True)
    executable = str(binary).replace("\\", "\\\\").replace('"', '\\"')
    executable = executable.replace("%", "%%")
    launcher = applications / "epub-reference-studio.desktop"
    launcher.write_text(
        "[Desktop Entry]\nType=Application\nName=EPUB Reference Studio\n"
        "Comment=Analyze and repair EPUB text against reviewed references\n"
        f'Exec="{executable}"\nIcon=epub-reference-studio\n'
        "Terminal=false\nCategories=Office;Literature;\n"
    )
    if shutil.which("update-desktop-database"):
        subprocess.run([
            "update-desktop-database", str(applications),
        ], check=True)
    print(f"Installed: {binary}\nApplication-menu entry: {launcher}")


if __name__ == "__main__":
    main()
