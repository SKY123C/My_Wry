use std::{
    any::Any,
    borrow::Cow,
    collections::HashMap,
    fs,
    panic::{self, AssertUnwindSafe},
    path::{Component, Path, PathBuf},
    sync::{Arc, Condvar, LazyLock, Mutex},
    thread,
};

use percent_encoding::percent_decode_str;
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    platform::{run_return::EventLoopExtRunReturn, windows::EventLoopBuilderExtWindows},
    window::{Icon, Window, WindowBuilder, WindowId},
};
use wry::{
    WebView, WebViewBuilder,
    http::{Request, Response, header::CONTENT_TYPE},
};

use crate::{Mode, middleware::Context};

#[derive(Default)]
struct InstanceStatus {
    running: bool,
    stop_requested: bool,
    visible: bool,
    last_error: Option<String>,
}

pub struct InstanceState {
    status: Mutex<InstanceStatus>,
    finished: Condvar,
}

impl InstanceState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            status: Mutex::new(InstanceStatus::default()),
            finished: Condvar::new(),
        })
    }
}

#[derive(Clone)]
struct AppConfig {
    id: u64,
    mode: Mode,
    resource_root: Option<PathBuf>,
    context: Arc<Context>,
    state: Arc<InstanceState>,
}

#[derive(Clone)]
enum RuntimeEvent {
    Create(AppConfig),
    Close {
        app_id: u64,
    },
    SetVisible {
        app_id: u64,
        visible: bool,
    },
    PythonResponse {
        app_id: u64,
        request_id: String,
        result_json: Option<String>,
        error: Option<String>,
    },
}

#[derive(Default)]
struct RuntimeState {
    running: bool,
    proxy: Option<EventLoopProxy<RuntimeEvent>>,
    pending: Vec<AppConfig>,
}

static RUNTIME: LazyLock<Mutex<RuntimeState>> =
    LazyLock::new(|| Mutex::new(RuntimeState::default()));
const TEST_BACKGROUND: &[u8] = include_bytes!("../assets/test-background.png");
const DEFAULT_WINDOW_ICON_PNG: &[u8] = include_bytes!("../assets/my_wry.png");
const TEST_HTML: &str = include_str!("../assets/test.html");
struct DecodedIcon {
    rgba: Vec<u8>,
    width: u32,
    height: u32,
}

static DEFAULT_WINDOW_ICON: LazyLock<Result<DecodedIcon, String>> = LazyLock::new(|| {
    let image =
        image::load_from_memory_with_format(DEFAULT_WINDOW_ICON_PNG, image::ImageFormat::Png)
            .map_err(|error| format!("无法解码默认窗口图标 assets/my_wry.png：{error}"))?
            .into_rgba8();
    let (width, height) = image.dimensions();
    Ok(DecodedIcon {
        rgba: image.into_raw(),
        width,
        height,
    })
});

struct WindowEntry {
    window: Window,
    webview: WebView,
    context: Arc<Context>,
    state: Arc<InstanceState>,
}

pub fn start(
    id: u64,
    mode: Mode,
    resource_root: Option<PathBuf>,
    context: Arc<Context>,
    state: Arc<InstanceState>,
) -> Result<(), String> {
    let resource_root = validate_resource_root(resource_root, &mode)?;
    {
        let mut status = state
            .status
            .lock()
            .map_err(|_| "无法锁定应用实例状态".to_owned())?;
        if status.running {
            return Err("这个 MyWryAPP 实例已经在运行".to_owned());
        }
        status.running = true;
        status.stop_requested = false;
        status.visible = true;
        status.last_error = None;
    }

    let config = AppConfig {
        id,
        mode,
        resource_root,
        context,
        state: Arc::clone(&state),
    };
    let mut runtime = RUNTIME
        .lock()
        .map_err(|_| "无法锁定 WRY 运行时状态".to_owned())?;
    if let Some(proxy) = runtime.proxy.clone() {
        return proxy.send_event(RuntimeEvent::Create(config)).map_err(|_| {
            finish_instance(&state, Some("WRY 运行时已退出".to_owned()));
            "无法向 WRY 线程发送创建窗口请求".to_owned()
        });
    }
    runtime.pending.push(config);
    if runtime.running {
        return Ok(());
    }
    runtime.running = true;
    if let Err(error) = thread::Builder::new()
        .name("wry-runtime".to_owned())
        .spawn(run_managed)
    {
        runtime.running = false;
        runtime.pending.retain(|pending| pending.id != id);
        finish_instance(&state, Some(format!("无法创建 WRY 线程：{error}")));
        return Err(format!("无法创建 WRY 线程：{error}"));
    }
    Ok(())
}

pub fn stop(id: u64, state: &Arc<InstanceState>) -> Result<(), String> {
    {
        let mut status = state
            .status
            .lock()
            .map_err(|_| "无法锁定应用实例状态".to_owned())?;
        if !status.running {
            return Ok(());
        }
        status.stop_requested = true;
    }
    if let Some(proxy) = RUNTIME
        .lock()
        .map_err(|_| "无法锁定 WRY 运行时状态".to_owned())?
        .proxy
        .clone()
    {
        let _ = proxy.send_event(RuntimeEvent::Close { app_id: id });
    }
    Ok(())
}

pub fn is_running(state: &InstanceState) -> Result<bool, String> {
    state
        .status
        .lock()
        .map(|status| status.running)
        .map_err(|_| "无法锁定应用实例状态".to_owned())
}

pub fn set_visible(id: u64, state: &InstanceState, visible: bool) -> Result<(), String> {
    {
        let mut status = state
            .status
            .lock()
            .map_err(|_| "无法锁定应用实例状态".to_owned())?;
        if !status.running {
            return Err("这个 MyWryAPP 实例尚未运行".to_owned());
        }
        status.visible = visible;
    }
    if let Some(proxy) = RUNTIME
        .lock()
        .map_err(|_| "无法锁定 WRY 运行时状态".to_owned())?
        .proxy
        .clone()
    {
        proxy
            .send_event(RuntimeEvent::SetVisible {
                app_id: id,
                visible,
            })
            .map_err(|_| "无法向 WRY 线程发送窗口可见性请求".to_owned())?;
    }
    Ok(())
}

pub fn wait(state: &InstanceState) -> Result<(), String> {
    let status = state
        .status
        .lock()
        .map_err(|_| "无法锁定应用实例状态".to_owned())?;
    let status = state
        .finished
        .wait_while(status, |status| status.running)
        .map_err(|_| "等待应用实例时状态锁损坏".to_owned())?;
    status.last_error.clone().map_or(Ok(()), Err)
}

pub fn send_python_response(
    app_id: u64,
    request_id: String,
    result_json: Option<String>,
    error: Option<String>,
) -> Result<(), String> {
    let proxy = RUNTIME
        .lock()
        .map_err(|_| "无法锁定 WRY 运行时状态".to_owned())?
        .proxy
        .clone()
        .ok_or_else(|| "WRY 运行时尚未运行".to_owned())?;
    proxy
        .send_event(RuntimeEvent::PythonResponse {
            app_id,
            request_id,
            result_json,
            error,
        })
        .map_err(|_| "无法将 Python 返回值发送到 WRY 线程".to_owned())
}

fn run_managed() {
    let result = panic::catch_unwind(AssertUnwindSafe(run_event_loop))
        .unwrap_or_else(|payload| Err(format!("WRY 线程发生异常：{}", panic_text(&payload))));
    if let Err(error) = &result {
        eprintln!("{error}");
    }

    let restart = if let Ok(mut runtime) = RUNTIME.lock() {
        runtime.proxy = None;
        if runtime.pending.is_empty() {
            runtime.running = false;
            false
        } else {
            true
        }
    } else {
        false
    };
    if restart
        && thread::Builder::new()
            .name("wry-runtime".to_owned())
            .spawn(run_managed)
            .is_err()
    {
        let pending = if let Ok(mut runtime) = RUNTIME.lock() {
            runtime.running = false;
            std::mem::take(&mut runtime.pending)
        } else {
            Vec::new()
        };
        for config in pending {
            finish_instance(
                &config.state,
                Some("旧 WRY 线程退出后无法启动新的运行时线程".to_owned()),
            );
        }
    }
}

fn run_event_loop() -> Result<(), String> {
    let mut builder = EventLoopBuilder::<RuntimeEvent>::with_user_event();
    builder.with_any_thread(true);
    let mut event_loop = builder.build();
    let proxy = event_loop.create_proxy();
    let pending = {
        let mut runtime = RUNTIME
            .lock()
            .map_err(|_| "无法锁定 WRY 运行时状态".to_owned())?;
        runtime.proxy = Some(proxy.clone());
        std::mem::take(&mut runtime.pending)
    };
    for config in pending {
        let _ = proxy.send_event(RuntimeEvent::Create(config));
    }

    let mut windows: HashMap<u64, WindowEntry> = HashMap::new();
    let mut window_ids: HashMap<WindowId, u64> = HashMap::new();
    event_loop.run_return(|event, target, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(RuntimeEvent::Create(config)) => {
                let stop_requested = config
                    .state
                    .status
                    .lock()
                    .map(|status| status.stop_requested)
                    .unwrap_or(true);
                if stop_requested {
                    finish_instance(&config.state, None);
                } else {
                    match create_window(target, &config) {
                        Ok(entry) => {
                            window_ids.insert(entry.window.id(), config.id);
                            windows.insert(config.id, entry);
                        }
                        Err(error) => finish_instance(&config.state, Some(error)),
                    }
                }
                if windows.is_empty() {
                    *control_flow = ControlFlow::Exit;
                }
            }
            Event::UserEvent(RuntimeEvent::Close { app_id }) => {
                if windows.len() == 1 && windows.contains_key(&app_id) {
                    begin_runtime_exit();
                }
                close_window(app_id, &mut windows, &mut window_ids);
                if windows.is_empty() {
                    *control_flow = ControlFlow::Exit;
                }
            }
            Event::UserEvent(RuntimeEvent::SetVisible { app_id, visible }) => {
                if let Some(entry) = windows.get(&app_id) {
                    entry.window.set_visible(visible);
                    if visible {
                        entry.window.set_focus();
                    }
                }
            }
            Event::UserEvent(RuntimeEvent::PythonResponse {
                app_id,
                request_id,
                result_json,
                error,
            }) => {
                if let Some(entry) = windows.get(&app_id) {
                    let request_id =
                        serde_json::to_string(&request_id).expect("序列化 request_id 不应失败");
                    let result = result_json.unwrap_or_else(|| "null".to_owned());
                    let error = serde_json::to_string(&error).expect("序列化错误信息不应失败");
                    let script = format!(
                        "globalThis.__resolvePythonCall?.({request_id}, {result}, {error});"
                    );
                    if let Err(error) = entry.webview.evaluate_script(&script) {
                        eprintln!("向页面返回 Python 执行结果失败：{error}");
                    }
                }
            }
            Event::WindowEvent {
                window_id,
                event: WindowEvent::CloseRequested,
                ..
            } => {
                if let Some(app_id) = window_ids.get(&window_id).copied() {
                    let intercept = windows.get(&app_id).is_some_and(|entry| {
                        entry.context.has_handler(crate::middleware::EXIT_ACTION)
                    });
                    if intercept {
                        if let Some(entry) = windows.get(&app_id)
                            && let Err(error) = crate::middleware::enqueue_action(
                                &entry.context,
                                crate::middleware::EXIT_ACTION,
                            )
                        {
                            eprintln!("调度 Python exit 回调失败：{error}");
                        }
                    } else {
                        if windows.len() == 1 {
                            begin_runtime_exit();
                        }
                        close_window(app_id, &mut windows, &mut window_ids);
                    }
                }
                if windows.is_empty() {
                    *control_flow = ControlFlow::Exit;
                }
            }
            _ => {}
        }
    });
    for (_, entry) in windows {
        finish_instance(&entry.state, None);
    }
    Ok(())
}

fn create_window(
    target: &tao::event_loop::EventLoopWindowTarget<RuntimeEvent>,
    config: &AppConfig,
) -> Result<WindowEntry, String> {
    let title = if config.mode == Mode::Test {
        "WRY 测试模式"
    } else {
        "WRY 示例"
    };
    let visible = config
        .state
        .status
        .lock()
        .map(|status| status.visible)
        .unwrap_or(true);
    let window = WindowBuilder::new()
        .with_title(title)
        .with_inner_size(tao::dpi::LogicalSize::new(800.0, 560.0))
        .with_visible(visible)
        .with_window_icon(Some(default_window_icon()?))
        .build(target)
        .map_err(|error| format!("无法创建原生窗口：{error}"))?;
    let is_test = config.mode == Mode::Test;
    let resource_root = config.resource_root.clone();
    let context = Arc::clone(&config.context);
    let builder = WebViewBuilder::new()
        .with_custom_protocol("wry".to_owned(), move |_id, request| {
            resource_response(&request, is_test, resource_root.as_deref())
        })
        .with_initialization_script(
            "window.runtimeMessage = '这段文字由 Rust 在页面加载前注入。';",
        );
    let builder = match &config.mode {
        Mode::Test => builder.with_url("wry://localhost/"),
        Mode::Normal => builder.with_url("wry://localhost/"),
    };
    let webview = builder
        .with_ipc_handler(move |request| {
            let message = request.body();
            if let Err(error) = crate::middleware::enqueue(&context, message) {
                eprintln!("调度 Python action `{message}` 失败：{error}");
            }
        })
        .build(&window)
        .map_err(|error| format!("无法创建 WebView：{error}"))?;
    Ok(WindowEntry {
        window,
        webview,
        context: Arc::clone(&config.context),
        state: Arc::clone(&config.state),
    })
}

fn default_window_icon() -> Result<Icon, String> {
    match &*DEFAULT_WINDOW_ICON {
        Ok(icon) => Icon::from_rgba(icon.rgba.clone(), icon.width, icon.height)
            .map_err(|error| format!("无法创建默认窗口图标：{error}")),
        Err(error) => Err(error.clone()),
    }
}

fn close_window(
    app_id: u64,
    windows: &mut HashMap<u64, WindowEntry>,
    window_ids: &mut HashMap<WindowId, u64>,
) {
    if let Some(entry) = windows.remove(&app_id) {
        window_ids.remove(&entry.window.id());
        finish_instance(&entry.state, None);
    }
}

fn finish_instance(state: &InstanceState, error: Option<String>) {
    if let Ok(mut status) = state.status.lock() {
        status.running = false;
        status.stop_requested = false;
        status.last_error = error;
        state.finished.notify_all();
    }
}

fn begin_runtime_exit() {
    if let Ok(mut runtime) = RUNTIME.lock() {
        runtime.proxy = None;
    }
}

fn panic_text(payload: &Box<dyn Any + Send>) -> &str {
    payload
        .downcast_ref::<String>()
        .map_or_else(
            || payload.downcast_ref::<&str>().copied(),
            |text| Some(text),
        )
        .unwrap_or("未知错误")
}

fn resource_response(
    request: &Request<Vec<u8>>,
    is_test_mode: bool,
    resource_root: Option<&Path>,
) -> Response<Cow<'static, [u8]>> {
    let path = request.uri().path();
    if is_test_mode {
        match path {
            "/" => return response(200, "text/html; charset=utf-8", TEST_HTML.as_bytes()),
            "/test-background.png" => return response(200, "image/png", TEST_BACKGROUND),
            _ => {}
        }
    }
    let Some(resource_root) = resource_root else {
        return response(404, "text/plain; charset=utf-8", b"Not Found");
    };
    match read_resource(resource_root, path) {
        Ok((content_type, body)) => response_owned(200, content_type, body),
        Err((status, message)) => {
            response_owned(status, "text/plain; charset=utf-8", message.into_bytes())
        }
    }
}

fn validate_resource_root(
    resource_root: Option<PathBuf>,
    mode: &Mode,
) -> Result<Option<PathBuf>, String> {
    let Some(resource_root) = resource_root else {
        return if *mode == Mode::Normal {
            Err("Normal 模式必须提供 resource_root，并且目录中必须包含 index.html".to_owned())
        } else {
            Ok(None)
        };
    };
    let canonical = fs::canonicalize(&resource_root)
        .map_err(|error| format!("无法访问资源根目录 `{}`：{error}", resource_root.display()))?;
    if !canonical.is_dir() {
        return Err(format!("资源根路径不是目录：{}", canonical.display()));
    }
    if *mode == Mode::Normal {
        let index = fs::canonicalize(canonical.join("index.html")).map_err(|error| {
            format!(
                "Normal 模式的资源根目录缺少可访问的 index.html：{}（{error}）",
                canonical.display()
            )
        })?;
        if !index.is_file() || !index.starts_with(&canonical) {
            return Err(format!(
                "Normal 模式的 index.html 必须是 resource_root 内的文件：{}",
                index.display()
            ));
        }
    }
    Ok(Some(canonical))
}

fn read_resource(
    resource_root: &Path,
    uri_path: &str,
) -> Result<(&'static str, Vec<u8>), (u16, String)> {
    let decoded = percent_decode_str(uri_path.trim_start_matches('/'))
        .decode_utf8()
        .map_err(|_| (400, "资源路径不是有效的 UTF-8".to_owned()))?;
    let relative = if decoded.is_empty() {
        Path::new("index.html")
    } else {
        Path::new(decoded.as_ref())
    };
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return Err((403, "不允许访问资源根目录之外的路径".to_owned()));
    }
    let mut candidate = resource_root.join(relative);
    if candidate.is_dir() {
        candidate = candidate.join("index.html");
    }
    let resolved = match fs::canonicalize(&candidate) {
        Ok(path) => path,
        Err(_) if relative.extension().is_none() => {
            fs::canonicalize(resource_root.join("index.html"))
                .map_err(|_| (404, format!("资源不存在：{uri_path}")))?
        }
        Err(_) => return Err((404, format!("资源不存在：{uri_path}"))),
    };
    if !resolved.starts_with(resource_root) || !resolved.is_file() {
        return Err((403, "不允许访问资源根目录之外的文件".to_owned()));
    }
    let content_type = content_type_for(&resolved);
    let body = fs::read(&resolved).map_err(|error| {
        (
            500,
            format!("读取资源 `{}` 失败：{error}", resolved.display()),
        )
    })?;
    Ok((content_type, body))
}

fn content_type_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json" | "map") => "application/json; charset=utf-8",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("wasm") => "application/wasm",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn response(
    status: u16,
    content_type: &'static str,
    body: &'static [u8],
) -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, content_type)
        .body(Cow::Borrowed(body))
        .expect("构建自定义协议响应不应失败")
}

fn response_owned(
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
) -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, content_type)
        .body(Cow::Owned(body))
        .expect("构建自定义协议响应不应失败")
}
