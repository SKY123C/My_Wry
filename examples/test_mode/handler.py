"""Edit this module while the test window is open to see hot reload."""

import threading

import my_wry
from application import app

LOAD_COUNT = globals().get("LOAD_COUNT", 0) + 1


@app.on("test")
def test(event: my_wry.Event) -> dict[str, object]:
    assert threading.current_thread() is threading.main_thread()
    print(f"测试回调：模块已加载 {LOAD_COUNT} 次", event.data)
    return {
        "success": True,
        "module_load_count": LOAD_COUNT,
        "message": "可以修改 handler.py，然后再次点击测试按钮",
    }
