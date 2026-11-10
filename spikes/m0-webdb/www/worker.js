// Dedicated worker hosting SQLite (OPFS sahpool needs a worker context).
import init, * as db from "./pkg/m0_webdb.js";

const ready = init();
self.onmessage = async ({ data: { id, fn, args } }) => {
  await ready;
  try {
    const value = await db[fn](...(args ?? []));
    self.postMessage({ id, value });
  } catch (e) {
    self.postMessage({ id, error: String(e) });
  }
};
