import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const source = readFileSync(new URL("../python_bridge.js", import.meta.url));
const bridge = await import(`data:text/javascript;base64,${source.toString("base64")}`);

test("Python push events are delivered repeatedly until unsubscribe", () => {
  const received = [];
  const unsubscribe = bridge.onPythonEvent("task.progress", (data) => {
    received.push(data.percent);
  });

  globalThis.__receivePythonEvent("task.progress", { percent: 10 });
  globalThis.__receivePythonEvent("task.progress", { percent: 20 });
  unsubscribe();
  globalThis.__receivePythonEvent("task.progress", { percent: 30 });

  assert.deepEqual(received, [10, 20]);
});

test("one-shot Python calls still resolve normally", async () => {
  let sent;
  globalThis.ipc = {
    postMessage(message) {
      sent = JSON.parse(message);
    },
  };

  const pending = bridge.invokePython("ping", {}, { timeoutMs: 0 });
  globalThis.__resolvePythonCall(sent.request_id, { ok: true });

  assert.deepEqual(await pending, { ok: true });
});

test("disposing the bridge removes push listeners", () => {
  let count = 0;
  bridge.onPythonEvent("task.progress", () => {
    count += 1;
  });
  bridge.disposePythonBridge();
  globalThis.__receivePythonEvent("task.progress", { percent: 50 });

  assert.equal(count, 0);
});
