"""Run every research script in order and tee the output to results/run_all.log."""
from __future__ import annotations

import runpy
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
SCRIPTS = sorted(p for p in HERE.glob("[0-9][0-9]_*.py"))


class Tee:
    def __init__(self, path: Path):
        self.f = open(path, "w")
        self.stdout = sys.stdout

    def write(self, s):
        self.stdout.write(s)
        self.f.write(s)

    def flush(self):
        self.stdout.flush()
        self.f.flush()


def main() -> None:
    (HERE.parent / "results").mkdir(exist_ok=True)
    sys.stdout = Tee(HERE.parent / "results" / "run_all.log")
    sys.path.insert(0, str(HERE))
    for s in SCRIPTS:
        t0 = time.time()
        runpy.run_path(str(s), run_name="__main__")
        print(f"\n[{s.name} done in {time.time() - t0:.0f}s]")


if __name__ == "__main__":
    main()
