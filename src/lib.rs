mod app;
mod middleware;

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    sync::{Arc, LazyLock},
};

use pyo3::{
    exceptions::{PyRuntimeError, PyValueError},
    prelude::*,
};

#[pyclass(eq, eq_int, from_py_object, module = "my_wry")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    Test,
    #[pyo3(name = "normal")]
    Normal,
}

static NEXT_APP_ID: AtomicU64 = AtomicU64::new(1);

fn next_app_id() -> u64 {
    NEXT_APP_ID.fetch_add(1, Ordering::Relaxed)
}

struct NativeApp {
    id: u64,
    context: Arc<middleware::Context>,
    state: Arc<app::InstanceState>,
}

impl NativeApp {
    fn new(reload_modules: bool) -> Self {
        let id = next_app_id();
        Self {
            id,
            context: middleware::Context::new(id, reload_modules),
            state: app::InstanceState::new(),
        }
    }
}

#[pyclass(name = "MyWryAPP", module = "my_wry")]
struct MyWryApp {
    mode: Mode,
    resource_root: Option<PathBuf>,
    native: NativeApp,
}

#[pymethods]
impl MyWryApp {
    #[new]
    #[pyo3(signature = (mode, resource_root=None), text_signature = "(mode, resource_root=None)")]
    fn new(mode: Mode, resource_root: Option<PathBuf>) -> Self {
        let reload_modules = mode == Mode::Test;
        Self {
            mode,
            resource_root,
            native: NativeApp::new(reload_modules),
        }
    }

    #[getter]
    fn mode(&self) -> Mode {
        self.mode.clone()
    }

    #[getter]
    fn resource_root(&self) -> Option<PathBuf> {
        self.resource_root.clone()
    }

    #[pyo3(text_signature = "($self)")]
    fn start(&self) -> PyResult<()> {
        app::start(
            self.native.id,
            self.mode.clone(),
            self.resource_root.clone(),
            Arc::clone(&self.native.context),
            Arc::clone(&self.native.state),
        )
        .map_err(PyRuntimeError::new_err)
    }

    #[pyo3(text_signature = "($self)")]
    fn stop(&self) -> PyResult<()> {
        app::stop(self.native.id, &self.native.state).map_err(PyRuntimeError::new_err)
    }

    #[pyo3(text_signature = "($self)")]
    fn hide(&self) -> PyResult<()> {
        app::set_visible(self.native.id, &self.native.state, false).map_err(PyRuntimeError::new_err)
    }

    #[pyo3(text_signature = "($self)")]
    fn show(&self) -> PyResult<()> {
        app::set_visible(self.native.id, &self.native.state, true).map_err(PyRuntimeError::new_err)
    }

    #[pyo3(text_signature = "($self)")]
    fn is_running(&self) -> PyResult<bool> {
        app::is_running(&self.native.state).map_err(PyRuntimeError::new_err)
    }

    #[pyo3(text_signature = "($self)")]
    fn wait(&self, py: Python<'_>) -> PyResult<()> {
        py.detach(|| app::wait(&self.native.state))
            .map_err(PyRuntimeError::new_err)
    }

    #[pyo3(text_signature = "($self)")]
    fn poll(&self, py: Python<'_>) -> PyResult<()> {
        middleware::poll(py)
    }

    #[pyo3(text_signature = "($self, action)")]
    fn on(&self, action: String) -> PyResult<middleware::ActionDecorator> {
        middleware::decorator(Arc::clone(&self.native.context), action)
    }

    #[pyo3(text_signature = "($self, function)")]
    fn exit(&self, py: Python<'_>, function: Py<PyAny>) -> PyResult<Py<PyAny>> {
        middleware::register(&self.native.context, middleware::EXIT_ACTION, py, function)
    }

    #[pyo3(text_signature = "($self, action)")]
    fn emit(&self, action: String) -> PyResult<()> {
        middleware::emit(&self.native.context, action)
    }

    #[pyo3(text_signature = "($self, event_name, data)")]
    fn publish(&self, py: Python<'_>, event_name: String, data: Py<PyAny>) -> PyResult<()> {
        let event_name = event_name.trim();
        if event_name.is_empty() {
            return Err(PyValueError::new_err("event_name 不能为空"));
        }
        if !app::is_running(&self.native.state).map_err(PyRuntimeError::new_err)? {
            return Err(PyRuntimeError::new_err("这个 MyWryAPP 实例尚未运行"));
        }
        let serialized: String = PyModule::import(py, "json")?
            .call_method1("dumps", (data.bind(py),))?
            .extract()?;
        let value: serde_json::Value = serde_json::from_str(&serialized)
            .map_err(|error| PyValueError::new_err(format!("推送数据不是有效的 JSON：{error}")))?;
        app::publish(self.native.id, event_name.to_owned(), value.to_string())
            .map_err(PyRuntimeError::new_err)
    }

    #[pyo3(text_signature = "($self)")]
    fn registered_actions(&self) -> PyResult<Vec<String>> {
        self.native.context.registered_actions()
    }

    #[pyo3(text_signature = "($self)")]
    fn clear_handlers(&self) -> PyResult<()> {
        self.native.context.clear()
    }

    fn __repr__(&self) -> String {
        format!(
            "MyWryAPP(mode={:?}, resource_root={:?})",
            self.mode, self.resource_root
        )
    }
}

static DEFAULT_APP: LazyLock<NativeApp> = LazyLock::new(|| NativeApp::new(false));

#[doc(hidden)]
pub fn run_standalone() -> Result<(), String> {
    DEFAULT_APP.context.set_reload_modules(true);
    app::start(
        DEFAULT_APP.id,
        Mode::Test,
        None,
        Arc::clone(&DEFAULT_APP.context),
        Arc::clone(&DEFAULT_APP.state),
    )?;
    app::wait(&DEFAULT_APP.state)
}

#[pyfunction]
#[pyo3(signature = (mode, resource_root=None), text_signature = "(mode, resource_root=None)")]
fn start(mode: Mode, resource_root: Option<PathBuf>) -> PyResult<()> {
    DEFAULT_APP.context.set_reload_modules(mode == Mode::Test);
    app::start(
        DEFAULT_APP.id,
        mode,
        resource_root,
        Arc::clone(&DEFAULT_APP.context),
        Arc::clone(&DEFAULT_APP.state),
    )
    .map_err(PyRuntimeError::new_err)
}

#[pyfunction]
fn stop() -> PyResult<()> {
    app::stop(DEFAULT_APP.id, &DEFAULT_APP.state).map_err(PyRuntimeError::new_err)
}

#[pyfunction]
fn is_running() -> PyResult<bool> {
    app::is_running(&DEFAULT_APP.state).map_err(PyRuntimeError::new_err)
}

#[pyfunction]
fn wait(py: Python<'_>) -> PyResult<()> {
    py.detach(|| app::wait(&DEFAULT_APP.state))
        .map_err(PyRuntimeError::new_err)
}

#[pyfunction]
fn on(action: String) -> PyResult<middleware::ActionDecorator> {
    middleware::decorator(Arc::clone(&DEFAULT_APP.context), action)
}

#[pyfunction]
fn emit(action: String) -> PyResult<()> {
    middleware::emit(&DEFAULT_APP.context, action)
}

#[pyfunction]
fn registered_actions() -> PyResult<Vec<String>> {
    DEFAULT_APP.context.registered_actions()
}

#[pyfunction]
fn clear_handlers() -> PyResult<()> {
    DEFAULT_APP.context.clear()
}

#[pymodule]
fn my_wry(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add("__version__", env!("CARGO_PKG_VERSION"))?;
    module.add_class::<Mode>()?;
    module.add_class::<MyWryApp>()?;
    module.add_class::<middleware::Event>()?;
    module.add_class::<middleware::ActionDecorator>()?;
    module.add_function(wrap_pyfunction!(start, module)?)?;
    module.add_function(wrap_pyfunction!(stop, module)?)?;
    module.add_function(wrap_pyfunction!(is_running, module)?)?;
    module.add_function(wrap_pyfunction!(wait, module)?)?;
    module.add_function(wrap_pyfunction!(on, module)?)?;
    module.add_function(wrap_pyfunction!(emit, module)?)?;
    module.add_function(wrap_pyfunction!(registered_actions, module)?)?;
    module.add_function(wrap_pyfunction!(clear_handlers, module)?)?;
    Ok(())
}
