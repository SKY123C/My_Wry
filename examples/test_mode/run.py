"""Run the hot-reloading test-mode example."""

import sys
import time
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
DIST_ROOT = PROJECT_ROOT / "dist_examples"

if not (DIST_ROOT / "my_wry.pyd").is_file():
    raise SystemExit(
        "请先在项目根目录执行：.\\build_pyd.ps1 -OutputDirectory dist_examples"
    )

sys.path.insert(0, str(DIST_ROOT))
from application import app  # noqa: E402
from handler import test as _test  # noqa: E402, F401


def main() -> None:
    app.start()
    print("测试模式已启动；修改 handler.py 后再次点击按钮即可验证热重载。")
    try:
        while app.is_running():
            time.sleep(0.05)
    finally:
        app.stop()
        app.wait()
        app.clear_handlers()


if __name__ == "__main__":
    main()
