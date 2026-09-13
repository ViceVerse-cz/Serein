// The passcode stays in the main frame until a bounded native query drains it.
(() => {
  "use strict";
  const page = "https://serein-captcha.verification.invalid/";
  if (window !== window.top || location.href !== page) return;
  const capability = "__SEREIN_CAPTCHA_CAPABILITY__";
  const opened = Date.now();
  let pending = null;
  let delivered = false;
  const active = () => window === window.top && location.href === page && Date.now() - opened <= 300000;
  Object.defineProperty(window, "__serein_captcha_take_" + capability.slice(0, -1), {
    value: () => {
      const value = active() ? pending : null;
      pending = null;
      return value;
    },
    writable: false,
    configurable: false,
  });
  Object.defineProperty(window, "ipc", {
    value: Object.freeze({ postMessage(value) {
      if (!active() || delivered || typeof value !== "string" || value.length > 8270 || !value.startsWith(capability) || !/^[\x21-\x7e]+$/.test(value)) return;
      delivered = true;
      pending = value;
    }}),
    writable: false,
    configurable: false,
  });
})();
