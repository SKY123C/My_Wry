import threading

import my_wry
from application import app

LOAD_COUNT = globals().get("LOAD_COUNT", 0) + 1


@app.on("test")
def test(event: my_wry.Event) -> dict[str, object]:
    assert threading.current_thread() is threading.main_thread()
    print(f"Python 主线程执行（模块第 {LOAD_COUNT} 次加载）", event.data)
    return {
        "success": True,
        "message": "测试完成",
        "module_load_count": LOAD_COUNT,
    }


@app.exit
def on_exit(event: my_wry.Event) -> None:
    app.hide()