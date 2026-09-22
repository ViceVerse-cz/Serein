(() => {
  "use strict";
  if (window !== window.top || !["https://serein-captcha.verification.invalid/", "serein-captcha://verification.invalid/"].includes(location.href)) return;
  const config = __SEREIN_CAPTCHA_CONFIG__;
  let finished = false;
  document.addEventListener("keydown", event => {
    if (event.key === "Escape") send("cancelled");
  });
  function send(kind, value = "") {
    if (finished) return;
    finished = true;
    window.ipc.postMessage(config.capability + kind + ":" + value);
  }
  document.addEventListener("DOMContentLoaded", () => {
    const status = document.getElementById("status");
    const button = document.getElementById("verify");
    const fail = () => {
      status.textContent = "Verification could not load. Close this window and try again, or open the invite in Discord.";
      send("error");
    };
    window.sereinCaptchaLoaded = () => {
      try {
        const widget = window.hcaptcha.render("captcha", {
          sitekey: config.sitekey,
          theme: config.dark ? "dark" : "light",
          size: config.invisible ? "invisible" : (window.innerWidth < 330 ? "compact" : "normal"),
          callback: value => {
            if (typeof value !== "string" || value.length < 1 || value.length > 8192 || !/^[\x21-\x7e]+$/.test(value)) return fail();
            status.textContent = "Verified. Returning to Serein…";
            send("verified", value);
          },
          "expired-callback": () => send("expired"),
          "chalexpired-callback": () => send("expired"),
          "error-callback": fail
        });
        // hCaptcha's native SDK uses setData for service-supplied enterprise rqdata:
        // https://github.com/hCaptcha/react-native-hcaptcha/blob/master/Hcaptcha.js
        if (config.rqdata) window.hcaptcha.setData(widget, {rqdata: config.rqdata});
        status.textContent = "Complete the check below to continue joining the server.";
        if (config.invisible) {
          button.hidden = false;
          button.addEventListener("click", () => {
            try {
              if (config.rqdata) window.hcaptcha.setData(widget, {rqdata: config.rqdata});
              window.hcaptcha.execute(widget);
            } catch (_) { fail(); }
          });
        }
      } catch (_) { fail(); }
    };
    const script = document.createElement("script");
    script.src = `https://js.hcaptcha.com/1/api.js?onload=sereinCaptchaLoaded&render=explicit&recaptchacompat=off&host=${config.sitekey}.react-native.hcaptcha.com`;
    script.async = true;
    script.onerror = fail;
    document.head.appendChild(script);
  }, {once: true});
})();
