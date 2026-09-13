// Offline callback contract only: never loads or solves a real provider challenge.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");
const template = fs.readFileSync(__dirname + "/captcha.js", "utf8");
function page({invisible = false, origin = "https://serein-captcha.verification.invalid", subframe = false, href = origin + "/"} = {}) {
  const listeners = {}, messages = [], calls = [];
  const nodes = Object.fromEntries(["status", "verify"].map(id => [id, {addEventListener: (name, cb) => listeners[id + name] = cb}]));
  let params;
  const window = {
    ipc: {postMessage: value => messages.push(value)},
    hcaptcha: {
      render: (_, options) => { params = options; return 7; },
      setData: (widget, data) => calls.push(["data", widget, data.rqdata]),
      execute: widget => calls.push(["execute", widget])
    }
  };
  window.top = subframe ? {} : window;
  const document = {
    addEventListener: (name, cb) => listeners[name] = cb,
    getElementById: id => nodes[id],
    createElement: () => ({}),
    head: {appendChild: script => calls.push(["script", script])}
  };
  const config = {capability: "random-capability:", sitekey: "service-sitekey", rqdata: 'quoted"data', dark: true, invisible};
  vm.runInNewContext(template.replace("__SEREIN_CAPTCHA_CONFIG__", JSON.stringify(config)), {window, document, location: {origin, href}});
  listeners.DOMContentLoaded?.();
  window.sereinCaptchaLoaded?.();
  return {listeners, messages, calls, params};
}
let current = page();
assert.equal(current.params.sitekey, "service-sitekey");
assert.equal(current.params.theme, "dark");
assert.equal(current.params.size, "normal");
assert.deepEqual(current.calls[1], ["data", 7, 'quoted"data']);
assert.equal(current.calls.some(call => call[0] === "execute"), false);
current.params.callback("synthetic-passcode");
current.params.callback("duplicate");
assert.deepEqual(current.messages, ["random-capability:verified:synthetic-passcode"]);
current = page({invisible: true});
assert.equal(current.calls.some(call => call[0] === "execute"), false);
current.listeners.verifyclick();
assert.deepEqual(current.calls.at(-1), ["execute", 7]);
for (const callback of ["expired-callback", "chalexpired-callback", "error-callback"]) {
  current = page(); current.params[callback]();
  assert.equal(current.messages.length, 1);
  assert.match(current.messages[0], /:(expired|error):$/);
}
for (const value of ["", "x".repeat(8193), "newline\n", "☃"]) {
  current = page(); current.params.callback(value);
  assert.deepEqual(current.messages, ["random-capability:error:"]);
}
current = page(); current.listeners.keydown({key: "Escape"});
assert.deepEqual(current.messages, ["random-capability:cancelled:"]);
assert.equal(page({subframe: true}).calls.length, 0);
assert.equal(page({origin: "https://discord.com"}).calls.length, 0);
console.log("Offline CAPTCHA callback checks passed.");

assert.ok(page({origin: "null", href: "serein-captcha://verification.invalid/"}).params);
assert.equal(page({href: "serein-captcha://verification.invalid/other"}).calls.length, 0);
