use pyo3::{
    exceptions::{PyRuntimeError, PyTypeError, PyValueError},
    prelude::*,
    types::PyAny,
};
use serde::Deserialize;

pub const EXIT_ACTION: &str = "exit";
use std::{
    collections::{HashMap, VecDeque},
    ffi::c_void,
    panic::{self, AssertUnwindSafe},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Deserialize)]
struct RawEvent {
    #[serde(default)]
    request_id: Option<String>,
    action: String,
    #[serde(default)]
    element_id: Option<String>,
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    data: HashMap<String, serde_json::Value>,
}

#[pyclass(module = "my_wry", frozen)]
pub struct Event {
    #[pyo3(get)]
    request_id: Option<String>,
    #[pyo3(get)]
    action: String,
    #[pyo3(get)]
    element_id: Option<String>,
    #[pyo3(get)]
    value: Option<String>,
    data: Py<PyAny>,
}

#[pymethods]
impl Event {
    #[getter]
    fn data(&self, py: Python<'_>) -> Py<PyAny> {
        self.data.clone_ref(py)
    }

    fn __repr__(&self) -> String {
        format!(
            "Event(action={:?}, element_id={:?}, request_id={:?})",
            self.action, self.element_id, self.request_id
        )
    }
}

#[derive(Default)]
struct PendingEvents {
    events: VecDeque<RawEvent>,
    scheduled: bool,
}

pub struct Context {
    app_id: u64,
    reload_modules: AtomicBool,
    handlers: Mutex<HashMap<String, Py<PyAny>>>,
    pending_events: Mutex<PendingEvents>,
}

impl Context {
    pub fn new(app_id: u64, reload_modules: bool) -> Arc<Self> {
        Arc::new(Self {
            app_id,
            reload_modules: AtomicBool::new(reload_modules),
            handlers: Mutex::new(HashMap::new()),
            pending_events: Mutex::new(PendingEvents::default()),
        })
    }

    pub fn set_reload_modules(&self, enabled: bool) {
        self.reload_modules.store(enabled, Ordering::Release);
    }

    pub fn registered_actions(&self) -> PyResult<Vec<String>> {
        let handlers = self
            .handlers
            .lock()
            .map_err(|_| PyRuntimeError::new_err("无法锁定事件处理器注册表"))?;
        let mut actions = handlers.keys().cloned().collect::<Vec<_>>();
        actions.sort_unstable();
        Ok(actions)
    }

    pub fn clear(&self) -> PyResult<()> {
        self.handlers
            .lock()
            .map_err(|_| PyRuntimeError::new_err("无法锁定事件处理器注册表"))?
            .clear();
        self.pending_events
            .lock()
            .map_err(|_| PyRuntimeError::new_err("无法锁定待处理事件队列"))?
            .events
            .clear();
        Ok(())
    }

    pub fn has_handler(&self, action: &str) -> bool {
        self.handlers
            .lock()
            .is_ok_and(|handlers| handlers.contains_key(action))
    }
}

#[pyclass(module = "my_wry")]
pub struct ActionDecorator {
    context: Arc<Context>,
    action: String,
}

#[pymethods]
impl ActionDecorator {
    fn __call__(&self, py: Python<'_>, function: Py<PyAny>) -> PyResult<Py<PyAny>> {
        register(&self.context, &self.action, py, function)
    }
}

pub fn register(
    context: &Context,
    action: &str,
    py: Python<'_>,
    function: Py<PyAny>,
) -> PyResult<Py<PyAny>> {
    if !function.bind(py).is_callable() {
        return Err(PyTypeError::new_err("装饰器只能用于可调用对象"));
    }
    context
        .handlers
        .lock()
        .map_err(|_| PyRuntimeError::new_err("无法锁定事件处理器注册表"))?
        .insert(action.to_owned(), function.clone_ref(py));
    Ok(function)
}

pub fn decorator(context: Arc<Context>, action: String) -> PyResult<ActionDecorator> {
    let action = action.trim();
    if action.is_empty() {
        return Err(PyValueError::new_err("action 不能为空"));
    }
    Ok(ActionDecorator {
        context,
        action: action.to_owned(),
    })
}

pub fn emit(context: &Arc<Context>, action: String) -> PyResult<()> {
    let action = action.trim();
    if action.is_empty() {
        return Err(PyValueError::new_err("action 不能为空"));
    }
    enqueue_event(
        context,
        RawEvent {
            request_id: None,
            action: action.to_owned(),
            element_id: None,
            value: None,
            data: HashMap::new(),
        },
    )
    .map_err(PyRuntimeError::new_err)
}

pub fn poll(py: Python<'_>) -> PyResult<()> {
    // SAFETY: Python 从主线程调用这个函数并持有 GIL；Py_MakePendingCalls 会处理
    // Py_AddPendingCall 提交到当前主解释器的待处理回调。
    let result = unsafe { pyo3::ffi::Py_MakePendingCalls() };
    if result == 0 {
        Ok(())
    } else {
        Err(PyErr::fetch(py))
    }
}

pub fn enqueue_action(context: &Arc<Context>, action: &str) -> Result<(), String> {
    enqueue_event(
        context,
        RawEvent {
            request_id: None,
            action: action.to_owned(),
            element_id: None,
            value: None,
            data: HashMap::new(),
        },
    )
}

pub fn enqueue(context: &Arc<Context>, message: &str) -> Result<(), String> {
    let mut event = serde_json::from_str::<RawEvent>(message)
        .map_err(|error| format!("无法解析 HTML 事件 JSON：{error}"))?;
    event.action = event.action.trim().to_owned();
    if event.action.is_empty() {
        return Err("HTML 事件 action 不能为空".to_owned());
    }
    enqueue_event(context, event)
}

fn enqueue_event(context: &Arc<Context>, event: RawEvent) -> Result<(), String> {
    let should_schedule = {
        let mut pending = context
            .pending_events
            .lock()
            .map_err(|_| "无法锁定待处理事件队列".to_owned())?;
        pending.events.push_back(event);
        if pending.scheduled {
            false
        } else {
            pending.scheduled = true;
            true
        }
    };
    if !should_schedule {
        return Ok(());
    }

    let argument = Arc::into_raw(Arc::clone(context))
        .cast_mut()
        .cast::<c_void>();
    // SAFETY: CPython 允许从任意 OS 线程调用 Py_AddPendingCall；裸指针持有一个 Arc 强引用。
    let result = unsafe { pyo3::ffi::Py_AddPendingCall(Some(run_pending_events), argument) };
    if result == 0 {
        return Ok(());
    }
    // SAFETY: 调用失败时 CPython 不接管 argument，必须收回 Arc。
    unsafe { drop(Arc::from_raw(argument.cast::<Context>())) };
    if let Ok(mut pending) = context.pending_events.lock() {
        pending.scheduled = false;
    }
    Err("Python 主线程的 Pending Call 队列已满".to_owned())
}

extern "C" fn run_pending_events(argument: *mut c_void) -> i32 {
    // SAFETY: argument 来自 Arc::into_raw，并且 pending call 只执行一次。
    let context = unsafe { Arc::from_raw(argument.cast::<Context>()) };
    let result = panic::catch_unwind(AssertUnwindSafe(|| dispatch_pending_events(&context)));
    if result.is_err() {
        eprintln!("处理 Python 主线程事件时发生 Rust panic");
        if let Ok(mut pending) = context.pending_events.lock() {
            pending.scheduled = false;
        }
    }
    0
}

fn dispatch_pending_events(context: &Arc<Context>) {
    loop {
        let events = {
            let Ok(mut pending) = context.pending_events.lock() else {
                eprintln!("无法锁定待处理事件队列");
                return;
            };
            if pending.events.is_empty() {
                pending.scheduled = false;
                return;
            }
            pending.events.drain(..).collect::<Vec<_>>()
        };
        Python::attach(|py| {
            for event in events {
                dispatch_one(py, context, event);
            }
        });
    }
}

fn dispatch_one(py: Python<'_>, context: &Arc<Context>, raw_event: RawEvent) {
    let request_id = raw_event.request_id.clone();
    let callback = match callback_for_action(py, context, &raw_event.action) {
        Ok(callback) => callback,
        Err(error) => {
            respond(
                context.app_id,
                request_id,
                None,
                Some(format!("读取 Python 回调失败：{error}")),
            );
            error.print(py);
            return;
        }
    };
    let Some(mut callback) = callback else {
        let message = format!("未注册 Python action：{}", raw_event.action);
        eprintln!("{message}");
        respond(context.app_id, request_id, None, Some(message));
        return;
    };
    if context.reload_modules.load(Ordering::Acquire) {
        callback = match reload_callback_module(py, context, &raw_event.action, &callback) {
            Ok(callback) => callback,
            Err(error) => {
                respond(
                    context.app_id,
                    request_id,
                    None,
                    Some(format!("重新加载 Python 模块失败：{error}")),
                );
                error.print(py);
                return;
            }
        };
    }
    let data = match event_data_to_python(py, &raw_event.data) {
        Ok(data) => data,
        Err(error) => {
            respond(
                context.app_id,
                request_id,
                None,
                Some(format!("无法转换事件数据：{error}")),
            );
            error.print(py);
            return;
        }
    };
    let event = match Py::new(
        py,
        Event {
            request_id: raw_event.request_id,
            action: raw_event.action,
            element_id: raw_event.element_id,
            value: raw_event.value,
            data,
        },
    ) {
        Ok(event) => event,
        Err(error) => {
            respond(
                context.app_id,
                request_id,
                None,
                Some(format!("无法创建 Python Event：{error}")),
            );
            error.print(py);
            return;
        }
    };
    match callback.call1(py, (event,)) {
        Ok(result) => match python_result_to_json(py, &result) {
            Ok(result_json) => respond(context.app_id, request_id, Some(result_json), None),
            Err(error) => {
                respond(
                    context.app_id,
                    request_id,
                    None,
                    Some(format!("Python 返回值无法序列化为 JSON：{error}")),
                );
                error.print(py);
            }
        },
        Err(error) => {
            respond(
                context.app_id,
                request_id,
                None,
                Some(format!("Python 回调执行失败：{error}")),
            );
            error.print(py);
        }
    }
}

fn callback_for_action(
    py: Python<'_>,
    context: &Context,
    action: &str,
) -> PyResult<Option<Py<PyAny>>> {
    let handlers = context
        .handlers
        .lock()
        .map_err(|_| PyRuntimeError::new_err("无法锁定事件处理器注册表"))?;
    Ok(handlers.get(action).map(|handler| handler.clone_ref(py)))
}

fn reload_callback_module(
    py: Python<'_>,
    context: &Context,
    action: &str,
    callback: &Py<PyAny>,
) -> PyResult<Py<PyAny>> {
    let module_name = callback
        .bind(py)
        .getattr("__module__")?
        .extract::<String>()?;
    let function_name = callback.bind(py).getattr("__name__")?.extract::<String>()?;
    if module_name == "__main__" {
        return Err(PyRuntimeError::new_err(
            "测试模式回调必须定义在可导入的独立模块中，不能定义在 __main__",
        ));
    }
    let module = PyModule::import(py, &module_name)?;
    let reloaded = PyModule::import(py, "importlib")?.call_method1("reload", (module,))?;
    let function = reloaded.getattr(&function_name)?.unbind();
    if !function.bind(py).is_callable() {
        return Err(PyTypeError::new_err(format!(
            "重载后的 `{module_name}.{function_name}` 不可调用"
        )));
    }
    context
        .handlers
        .lock()
        .map_err(|_| PyRuntimeError::new_err("无法锁定事件处理器注册表"))?
        .insert(action.to_owned(), function.clone_ref(py));
    Ok(function)
}

fn respond(
    app_id: u64,
    request_id: Option<String>,
    result_json: Option<String>,
    error: Option<String>,
) {
    let Some(request_id) = request_id else {
        return;
    };
    if let Err(send_error) =
        crate::app::send_python_response(app_id, request_id, result_json, error)
    {
        eprintln!("{send_error}");
    }
}

fn python_result_to_json(py: Python<'_>, result: &Py<PyAny>) -> PyResult<String> {
    PyModule::import(py, "json")?
        .call_method1("dumps", (result.bind(py),))?
        .extract()
}

fn event_data_to_python(
    py: Python<'_>,
    data: &HashMap<String, serde_json::Value>,
) -> PyResult<Py<PyAny>> {
    let serialized = serde_json::to_string(data)
        .map_err(|error| PyValueError::new_err(format!("无法序列化事件数据：{error}")))?;
    Ok(PyModule::import(py, "json")?
        .call_method1("loads", (serialized,))?
        .unbind())
}
