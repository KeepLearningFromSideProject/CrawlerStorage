import shutil
import subprocess
import sys
import time
from pathlib import Path

db_path = Path("data.db")
backup_path = Path("data.db.bak")

try:
    shutil.copy(db_path, backup_path)
except:
    sys.exit(1)

try:
    with open(db_path, "wb") as db:
        db.write(b"")
    subprocess.run(["diesel", "migration", "run"])
    fs_daemon = subprocess.Popen(
        ["target/debug/comic-fs", "mnt"],
        env={"RUST_BACKTRACE": "1", "RUST_LOG": "debug"},
    )
    time.sleep(3)
    path = Path("mnt/comics/my-comic/ep1/001.jpg")
    path.parent.mkdir(parents=True, exist_ok=True)
    with open(path, "wb") as f:
        f.write(b"123")
    with open(path, "rb") as f:
        assert f.read() == b"123"
    with open(path, "wb") as f:
        f.write(b"456")
    with open(path, "rb") as f:
        assert f.read() == b"456"
except Exception as e:
    print(e)
    sys.exit(1)
finally:
    subprocess.run(["fusermount", "-u", "mnt"])
    shutil.move(backup_path, db_path)
