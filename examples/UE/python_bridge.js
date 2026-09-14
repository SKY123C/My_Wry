const pendingRequests = new Map();
const eventListeners = new Map();

export class PythonBridgeError extends Error {
  constructor(message, action = null) {
    super(message);
    this.name = "PythonBridgeError";
    this.action = action;
  }
}

function createRequestId() {
  if (globalThis.crypto?.randomUUID) {
    return globalThis.crypto.randomUUID();
  }

  return `${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

export function invokePython(action, data = {}, options = {}) {
  if (typeof action !== "string" || action.trim() === "") {
    return Promise.reject(new PythonBridgeError("action 必须是非空字符串"));
  }

  const requestId = createRequestId();
  const timeoutMs = options.timeoutMs ?? 10000;

  return new Promise((resolve, reject) => {
    const timeoutId = timeoutMs > 0
      ? globalThis.setTimeout(() => {
          if (pendingRequests.delete(requestId)) {
            reject(new PythonBridgeError(`Python 调用超时：${action}`, action));
          }
        }, timeoutMs)
      : null;

    pendingRequests.set(requestId, { action, resolve, reject, timeoutId });

    try {
      if (!globalThis.ipc?.postMessage) {
        throw new PythonBridgeError("当前页面没有可用的 WRY IPC", action);
      }

      globalThis.ipc.postMessage(JSON.stringify({
        request_id: requestId,
        action,
        element_id: options.elementId ?? null,
        value: options.value ?? null,
        data,
      }));
    } catch (error) {
      settleRequest(requestId, null, error instanceof Error ? error.message : String(error));
    }
  });
}

export function resolvePythonCall(requestId, result = null, error = null) {
  settleRequest(requestId, result, error);
}

export function onPythonEvent(eventName, listener) {
  if (typeof eventName !== "string" || eventName.trim() === "") {
    throw new PythonBridgeError("eventName 必须是非空字符串");
  }
  if (typeof listener !== "function") {
    throw new PythonBridgeError("listener 必须是函数");
  }

  const name = eventName.trim();
  let listeners = eventListeners.get(name);
  if (!listeners) {
    listeners = new Set();
    eventListeners.set(name, listeners);
  }
  listeners.add(listener);

  return () => {
    listeners.delete(listener);
    if (listeners.size === 0) {
      eventListeners.delete(name);
    }
  };
}

export function receivePythonEvent(eventName, data) {
  const listeners = eventListeners.get(eventName);
  if (!listeners) {
    return false;
  }
  for (const listener of [...listeners]) {
    try {
      listener(data);
    } catch (error) {
      globalThis.console?.error?.("Python 事件监听器执行失败：", error);
    }
  }
  return true;
}

export function disposePythonBridge(reason = "Python bridge 已关闭") {
  for (const requestId of [...pendingRequests.keys()]) {
    settleRequest(requestId, null, reason);
  }
  eventListeners.clear();
}

function settleRequest(requestId, result, error) {
  const pending = pendingRequests.get(requestId);
  if (!pending) {
    return false;
  }

  pendingRequests.delete(requestId);
  if (pending.timeoutId !== null) {
    globalThis.clearTimeout(pending.timeoutId);
  }

  if (error !== null && error !== undefined) {
    pending.reject(new PythonBridgeError(String(error), pending.action));
  } else {
    pending.resolve(result);
  }
  return true;
}

// Rust 通过 WebView.evaluate_script() 调用这个稳定的全局入口。
globalThis.__resolvePythonCall = resolvePythonCall;
globalThis.__receivePythonEvent = receivePythonEvent;
