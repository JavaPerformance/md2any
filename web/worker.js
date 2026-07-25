/**
 * md2any Studio engine worker — keeps WASM off the UI thread.
 * Protocol: { id, op, ...args } → { id, ok, result? , error? }
 */
import init, {
  version,
  themeNames,
  layoutNames,
  previewWindow,
  convert,
  lint,
  extractBrand,
  slideImages,
} from "./pkg/md2any_wasm.js";

let ready;
function ensureInit() {
  if (!ready) {
    ready = init().then(() => true);
  }
  return ready;
}

/** Memoize identical previewWindow calls (same md+opts+window). */
const previewMemo = new Map();
const PREVIEW_MEMO_MAX = 8;

function previewKey(msg) {
  return [
    msg.markdown ?? "",
    msg.theme ?? "",
    msg.aspect ?? "",
    msg.layout ?? "",
    msg.assetsJson ?? "",
    msg.htmlFrom >>> 0,
    msg.htmlTo >>> 0,
  ].join("\x1e");
}

function memoGet(key) {
  const hit = previewMemo.get(key);
  if (!hit) return null;
  // LRU touch
  previewMemo.delete(key);
  previewMemo.set(key, hit);
  return hit;
}

function memoSet(key, value) {
  if (previewMemo.has(key)) previewMemo.delete(key);
  previewMemo.set(key, value);
  while (previewMemo.size > PREVIEW_MEMO_MAX) {
    const first = previewMemo.keys().next().value;
    previewMemo.delete(first);
  }
}

self.onmessage = async (ev) => {
  const msg = ev.data || {};
  const id = msg.id;
  try {
    await ensureInit();
    let result;
    switch (msg.op) {
      case "init":
        result = {
          version: version(),
          themes: themeNames(),
          layouts: layoutNames(),
        };
        break;
      case "previewWindow": {
        const key = previewKey(msg);
        const cached = memoGet(key);
        if (cached) {
          result = cached;
          break;
        }
        result = previewWindow(
          msg.markdown ?? "",
          msg.theme ?? null,
          msg.aspect ?? null,
          msg.layout ?? null,
          msg.assetsJson ?? null,
          msg.htmlFrom >>> 0,
          msg.htmlTo >>> 0
        );
        memoSet(key, result);
        break;
      }
      case "convert":
        // Export invalidates preview memo (content may have side-effects).
        previewMemo.clear();
        result = convert(
          msg.markdown ?? "",
          msg.format ?? "pdf",
          msg.theme ?? null,
          msg.aspect ?? null,
          msg.layout ?? null,
          msg.assetsJson ?? null
        );
        break;
      case "lint":
        result = lint(
          msg.markdown ?? "",
          msg.theme ?? null,
          msg.aspect ?? null,
          msg.layout ?? null,
          msg.assetsJson ?? null
        );
        break;
      case "extractBrand":
        result = extractBrand(msg.base64 ?? "", msg.sourceName ?? null);
        break;
      case "slideImages":
        result = slideImages(
          msg.markdown ?? "",
          msg.theme ?? null,
          msg.aspect ?? null,
          msg.layout ?? null,
          msg.assetsJson ?? null,
          msg.from >>> 0,
          msg.to >>> 0,
          msg.format ?? "svg"
        );
        break;
      default:
        throw new Error(`unknown op: ${msg.op}`);
    }
    self.postMessage({ id, ok: true, result });
  } catch (e) {
    self.postMessage({
      id,
      ok: false,
      error: e && e.message ? e.message : String(e),
    });
  }
};
