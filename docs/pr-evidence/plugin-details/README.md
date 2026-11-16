# Plugin details and native settings — synthetic native evidence

Both captures use `target/debug/serein --demo --demo-settings=extensions`, C.UTF-8 locale and
1120×760 crops from separate isolated Xvfb displays with separate synthetic HOME directories.
`before.png` was captured from main; `after.png` was captured after clicking the visible **Details**
button for API Proxy with an X11 input event. No account data, proxy address, credential or third-party
authorization was entered or submitted.

The dialog displays ordinary metadata, declared capabilities and disabled-until-enabled native actions.
The fixture has no catalog description in this checkout, so it honestly shows “No description provided.”
The independently generated catalog descriptions populate that existing field after their own merge;
no second description store was introduced or duplicated in this screenshot.

Focused tests additionally cover invocation/action-value routing, disabled and empty states,
cleanup/reload, stale result handling and specialized native editors. The offline `extension_settings`
navigation example currently stops at an existing navigation assertion before reaching details, so it
was not counted as evidence of this feature. No synthetic evidence here proves a live Discord service
interaction or that a paid/authorized connection exists.
