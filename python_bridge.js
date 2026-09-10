const pendingRequests = new Map();

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

export function disposePythonBridge(reason = "Python bridge 已关闭") {
  for (const requestId of [...pendingRequests.keys()]) {
    settleRequest(requestId, null, reason);
  }
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
