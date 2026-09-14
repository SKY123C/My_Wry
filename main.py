import sys

sys.path.append(r"C:\data\my_wry\dist")
import threading
import time

import my_wry

app = my_wry.MyWryAPP(my_wry.Mode.Test)

@my_wry.on("test")
def test(event: my_wry.Event) -> dict[str, object]:
    assert threading.current_thread() is threading.main_thread()
    print("Python 主线程执行", event.action, event.data)
    return {"success": True, "message": "测试完成"}


my_wry.start(my_wry.Mode.Test)

while my_wry.is_running():
    time.sleep(0.05)

my_wry.wait()
my_wry.clear_handlers()
