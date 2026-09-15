"""Launch the WRY content browser from Unreal Editor's Python console."""

import importlib
import re
import sys
from pathlib import Path
from types import ModuleType
from typing import Optional

ROOT = Path(__file__).resolve().parent
PROJECT_ROOT = ROOT.parents[1]
sys.path.insert(0, str(PROJECT_ROOT / "dist"))
import my_wry

app = my_wry.MyWryAPP(my_wry.Mode.normal, resource_root=ROOT)
_NAME = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
_slate_tick_handle: Optional[object] = None


def _unreal() -> ModuleType:
    return importlib.import_module("unreal")


def _folder(value: object) -> str:
    if not isinstance(value, str) or not (value == "/Game" or value.startswith("/Game/")):
        raise ValueError("目录必须位于 /Game 下")
    return value.rstrip("/")


def _asset_path(value: object) -> str:
    if not isinstance(value, str) or not value.startswith("/Game/"):
        raise ValueError("资产路径必须位于 /Game 下")
    return value


@app.on("browser.environment")
def environment(_event: my_wry.Event) -> dict[str, object]:
    unreal = _unreal()
    return {"project": str(unreal.SystemLibrary.get_game_name())}


@app.on("browser.list")
def list_directory(event: my_wry.Event) -> dict[str, object]:
    unreal = _unreal()
    folder = _folder(event.data.get("folder", "/Game"))
    registry = unreal.AssetRegistryHelpers.get_asset_registry()
    folders = [
        {"path": path, "name": path.rsplit("/", 1)[-1]}
        for path in sorted(
            (str(raw).rstrip("/") for raw in registry.get_sub_paths(folder, False)),
            key=str.lower,
        )
    ]
    entries = unreal.EditorAssetLibrary.list_assets(
        folder, recursive=False, include_folder=False
    )
    assets: list[dict[str, str]] = []
    for raw in entries[:500]:
        path = str(raw)
        asset_data = unreal.EditorAssetLibrary.find_asset_data(path)
        class_path = getattr(asset_data, "asset_class_path", None)
        asset_class = str(getattr(class_path, "asset_name", "")) or str(
            getattr(asset_data, "asset_class", "Asset")
        )
        name = path.rsplit("/", 1)[-1].split(".", 1)[0]
        assets.append({"path": path, "name": name, "class": asset_class})
    assets.sort(key=lambda item: item["name"].lower())
    return {"folder": folder, "folders": folders, "assets": assets, "truncated": len(entries) > 500}


@app.on("browser.tree")
def directory_tree(_event: my_wry.Event) -> dict[str, object]:
    unreal = _unreal()
    registry = unreal.AssetRegistryHelpers.get_asset_registry()
    paths = sorted(
        {str(raw).rstrip("/") for raw in registry.get_sub_paths("/Game", True)},
        key=str.lower,
    )
    return {"paths": paths}


@app.on("browser.select")
def select_asset(event: my_wry.Event) -> dict[str, object]:
    unreal = _unreal()
    path = _asset_path(event.data.get("path"))
    if not unreal.EditorAssetLibrary.does_asset_exist(path):
        raise FileNotFoundError(path)
    unreal.EditorAssetLibrary.sync_browser_to_objects([path])
    return {"selected": path}


@app.on("browser.create")
def create_asset(event: my_wry.Event) -> dict[str, object]:
    unreal = _unreal()
    folder = _folder(event.data.get("folder"))
    name = event.data.get("name")
    kind = event.data.get("kind")
    if not isinstance(name, str) or not _NAME.fullmatch(name):
        raise ValueError("名称只能包含英文字母、数字和下划线，且不能以数字开头")
    if kind == "material":
        asset_class, factory = unreal.Material, unreal.MaterialFactoryNew()
    elif kind == "blueprint":
        factory = unreal.BlueprintFactory()
        factory.set_editor_property("parent_class", unreal.Actor)
        asset_class = unreal.Blueprint
    else:
        raise ValueError("不支持的资产类型")
    if not unreal.EditorAssetLibrary.does_directory_exist(folder):
        raise FileNotFoundError(folder)
    package = f"{folder}/{name}"
    if unreal.EditorAssetLibrary.does_asset_exist(package):
        raise FileExistsError(package)
    created = unreal.AssetToolsHelpers.get_asset_tools().create_asset(
        name, folder, asset_class, factory
    )
    if created is None:
        raise RuntimeError("UE 未能创建资产")
    path = str(created.get_path_name())
    unreal.EditorAssetLibrary.sync_browser_to_objects([path])
    return {"path": path, "name": name}


@app.on("browser.create_folder")
def create_folder(event: my_wry.Event) -> dict[str, object]:
    unreal = _unreal()
    parent = _folder(event.data.get("folder"))
    name = event.data.get("name")
    if not isinstance(name, str) or not _NAME.fullmatch(name):
        raise ValueError("目录名只能包含英文字母、数字和下划线")
    path = f"{parent}/{name}"
    if unreal.EditorAssetLibrary.does_directory_exist(path):
        raise FileExistsError(path)
    if not unreal.EditorAssetLibrary.make_directory(path):
        raise RuntimeError("UE 未能创建目录")
    return {"path": path}


def _poll_wry_events(_delta_seconds: float) -> None:
    """Drive CPython pending calls from UE's main-thread Slate tick."""
    app.poll()


def _unregister_slate_tick() -> None:
    global _slate_tick_handle
    if _slate_tick_handle is not None:
        _unreal().unregister_slate_post_tick_callback(_slate_tick_handle)
        _slate_tick_handle = None


@app.exit
def close_app(_event: my_wry.Event) -> None:
    """Stop polling and close the WRY window when its close button is pressed."""
    _unregister_slate_tick()
    app.stop()


def start() -> None:
    """Register the UE main-thread pump and start the non-blocking WRY window."""
    global _slate_tick_handle
    if _slate_tick_handle is None:
        _slate_tick_handle = _unreal().register_slate_post_tick_callback(
            _poll_wry_events
        )
    try:
        app.start()
    except Exception:
        _unregister_slate_tick()
        raise


def stop() -> None:
    """Stop the WRY window and unregister its UE Slate callback."""
    _unregister_slate_tick()
    app.stop()


start()
