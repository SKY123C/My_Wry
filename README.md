# WRY 示例

一个最小的桌面 WebView 应用，展示：

- 使用 Rust 创建原生窗口并加载内嵌 HTML；
- 在页面加载前由 Rust 注入 JavaScript 数据；
- 通过 `window.ipc.postMessage` 从页面向 Rust 发送消息。

运行：

```powershell
cargo run
```

点击“向 Rust 发送消息”后，Rust 会在启动应用的终端打印收到的内容。

## 构建 Python 扩展

扩展使用 PyO3 的 `abi3-py39` 稳定 ABI，最低支持 CPython 3.9。可以继续使用当前的 Python 3.11.8 环境构建，生成的 `.pyd` 可由 CPython 3.9 及更高版本加载。

```powershell
.\build_pyd.ps1
```

产物位于 `dist\my_wry.pyd`。推荐为每个窗口创建一个 `MyWryAPP` 实例。`start()` 会在共享的 Rust WRY 线程中创建窗口并立即返回，不会阻塞 Python 主线程：

```python
import my_wry

app = my_wry.MyWryAPP(my_wry.Mode.Test)
app.start()

print(app.is_running())

# 需要退出窗口时调用，不会阻塞。
app.stop()

# 如需确认窗口线程已经完全结束：
app.wait()
```

每个实例都有独立的页面配置、运行状态、事件队列和 Python 回调注册表。多个实例共享同一个 Tao/WRY 事件循环线程，因此同一进程可以同时打开多个窗口，同名 action 也不会串线。多次调用同一实例的 `stop()` 是安全的。

`hide()` 只隐藏窗口，WebView 和回调仍然运行；`show()` 会重新显示窗口并获取焦点。可以用 `@app.exit` 接管原生窗口的关闭按钮：

```python
@app.exit
def on_exit(_event: my_wry.Event) -> None:
    app.hide()
```

注册 `@app.exit` 后，关闭按钮不会自动销毁窗口。回调在 Python 主线程执行，可以调用 `app.hide()` 实现最小化到后台，也可以调用 `app.stop()` 真正退出。没有注册退出回调时，关闭按钮保持默认退出行为。

`start()` 必须传入页面模式：

- `my_wry.Mode.Test`：测试模式，背景图铺满 WebView 内容区域，只显示一个 `data-action="test"` 按钮；
- `my_wry.Mode.normal`：普通模式，目前暂时沿用原页面。

测试模式每次收到按钮事件时，都会根据装饰器函数的 `__module__` 调用 `importlib.reload()`，然后执行重新注册后的最新函数。测试回调必须定义在 `test.py` 这类可导入的独立模块中，不能直接定义在 `__main__`。

普通模式必须通过 `resource_root` 提供前端框架的构建产物，并且该目录必须包含 `index.html`：

```python
app = my_wry.MyWryAPP(
    my_wry.Mode.normal,
    resource_root=r"C:\project\frontend\dist",
)
app.start()
```

资源根目录的 `index.html` 映射到 `wry://localhost/`，HTML 中的相对 JS、CSS、图片、字体和 WASM 路径会从同一目录读取。无扩展名的缺失路径会回退到 `index.html`，可用于 React Router 或 Vue Router。Normal 模式省略 `resource_root` 或找不到 `index.html` 时，`start()` 会直接抛出异常。

## 注册按钮事件

使用实例的 `on()` 装饰器将 HTML 的 `data-action` 映射到 Python 函数：

测试模式的可重载回调应放在独立模块中。`application.py` 保存实例：

```python
import my_wry

app = my_wry.MyWryAPP(my_wry.Mode.Test)
```

`test.py` 在该实例上注册回调：

```python
import threading

import my_wry
from application import app


@app.on("test")
def test(event: my_wry.Event) -> dict[str, object]:
    assert threading.current_thread() is threading.main_thread()
    return {"created": True, "input": event.data}
```

入口模块导入回调后启动实例：

```python
import time

from application import app
from test import test as _test

app.start()
while app.is_running():
    time.sleep(0.05)

app.wait()
app.clear_handlers()
```

事件链路为 `HTML data-action → WRY IPC → Rust 事件队列 → CPython 主线程 → Python 函数`。每个回调都会收到一个 `Event` 对象，其只读属性包括 `action`、`element_id`、`value` 和 `data`。回调默认在 Python 主线程执行，但主线程必须保持正常运行，不能永久阻塞在不返回 Python 的原生函数中。当前实现面向 CPython 主解释器，不支持在子解释器中注册回调。

嵌入式宿主销毁 Python 解释器之前，必须先调用 `stop()` 和 `wait()`，随后可用 `clear_handlers()` 释放注册函数。

## 前端调用 Python

`python_bridge.js` 是与框架无关的 ES Module，可以在任意前端项目中导入：

```javascript
import { invokePython } from "./python_bridge.js";

const result = await invokePython("create_project", {
  name: "示例项目"
});

console.log(result.created);
```

`invokePython()` 返回 Promise。模块会生成 `request_id`、管理并发请求和十秒超时，并安装供 Rust 返回结果使用的 `globalThis.__resolvePythonCall`。Python 返回值必须能够被 `json.dumps()` 序列化；Python 异常和序列化错误会转换成 Promise rejection。
