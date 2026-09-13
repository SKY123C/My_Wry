from collections.abc import Callable
from os import PathLike
from typing import ClassVar, Optional, TypeVar, Union, final

__version__: str


@final
class Mode:
    Test: ClassVar[Mode]
    normal: ClassVar[Mode]


@final
class Event:
    @property
    def request_id(self) -> Optional[str]: ...

    @property
    def action(self) -> str: ...

    @property
    def element_id(self) -> Optional[str]: ...

    @property
    def value(self) -> Optional[str]: ...

    @property
    def data(self) -> dict[str, object]: ...


_Callback = TypeVar("_Callback", bound=Callable[[Event], object])


class ActionDecorator:
    def __call__(self, function: _Callback) -> _Callback: ...


@final
class MyWryAPP:
    def __init__(
        self,
        mode: Mode,
        resource_root: Optional[Union[str, PathLike[str]]] = None,
    ) -> None: ...

    @property
    def mode(self) -> Mode: ...

    @property
    def resource_root(self) -> Optional[PathLike[str]]: ...

    def start(self) -> None:
        """在共享的 Rust WRY 线程中创建窗口；Normal 模式要求资源根目录含有 index.html。"""
        ...

    def stop(self) -> None:
        """请求关闭此实例的窗口并立即返回。"""
        ...

    def hide(self) -> None:
        """隐藏窗口，但保持实例和 WebView 运行。"""
        ...

    def show(self) -> None:
        """显示窗口并将其置于焦点。"""
        ...

    def is_running(self) -> bool: ...
    def wait(self) -> None: ...
    def on(self, action: str) -> ActionDecorator: ...
    def exit(self, function: _Callback) -> _Callback:
        """装饰关闭请求回调；注册后由该回调接管窗口关闭按钮。"""
        ...

    def emit(self, action: str) -> None: ...
    def publish(self, event_name: str, data: object) -> None:
        """从 Python 向当前实例的页面推送一条 JSON 事件。"""
        ...

    def registered_actions(self) -> list[str]: ...
    def clear_handlers(self) -> None: ...


def start(
    mode: Mode,
    resource_root: Optional[Union[str, PathLike[str]]] = None,
) -> None:
    """在 Rust 专用线程中启动 WRY 窗口并立即返回。"""
    ...


def stop() -> None:
    """请求关闭 WRY 窗口并立即返回。"""
    ...


def is_running() -> bool:
    """返回 WRY 窗口线程是否正在运行。"""
    ...


def wait() -> None:
    """等待 WRY 窗口线程结束。"""
    ...


def on(action: str) -> ActionDecorator:
    """注册与 HTML data-action 对应的单参数 Python 回调。"""
    ...


def emit(action: str) -> None:
    """向 Python 主线程事件队列提交一个 action。"""
    ...


def registered_actions() -> list[str]:
    """返回已注册的 action 名称。"""
    ...


def clear_handlers() -> None:
    """清空回调注册表和尚未处理的事件。"""
    ...
