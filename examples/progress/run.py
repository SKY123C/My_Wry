"""Run the server-push progress bar example with the PyO3 extension."""

import shutil
import sys
import tempfile
import threading
import time
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
EXAMPLE_ROOT = Path(__file__).resolve().parent
DIST_ROOT = PROJECT_ROOT / "dist_examples"

if not (DIST_ROOT / "my_wry.pyd").is_file():
    raise SystemExit(
        "请先在项目根目录执行：.\\build_pyd.ps1 -OutputDirectory dist_examples"
    )

sys.path.insert(0, str(DIST_ROOT))
import my_wry  # noqa: E402


def main() -> None:
    with tempfile.TemporaryDirectory(prefix="my_wry_progress_") as temp_dir:
        resource_root = Path(temp_dir)
        shutil.copy2(EXAMPLE_ROOT / "index.html", resource_root / "index.html")
        shutil.copy2(PROJECT_ROOT / "python_bridge.js", resource_root / "python_bridge.js")

        app = my_wry.MyWryAPP(my_wry.Mode.normal, resource_root=resource_root)
        stopping = threading.Event()

        def run_progress(task_id: str) -> None:
            for percent in range(0, 101, 5):
                if stopping.is_set():
                    return
                try:
                    app.publish(
                        "task.progress",
                        {
                            "task_id": task_id,
                            "percent": percent,
                            "done": percent == 100,
                        },
                    )
                except RuntimeError:
                    return  # The window has already been closed.
                if percent < 100 and stopping.wait(0.2):
                    return

        @app.on("start_task")
        def start_task(event: my_wry.Event) -> dict[str, object]:
            task_id = event.data.get("task_id")
            if not isinstance(task_id, str) or not task_id:
                raise ValueError("task_id 必须是非空字符串")
            threading.Thread(
                target=run_progress,
                args=(task_id,),
                daemon=True,
            ).start()
            return {"task_id": task_id, "started": True}

        app.start()
        print("进度条示例已启动；点击页面按钮，关闭窗口即可退出。")
        try:
            while app.is_running():
                time.sleep(0.05)
        finally:
            stopping.set()
            app.stop()
            app.wait()
            app.clear_handlers()


if __name__ == "__main__":
    main()
