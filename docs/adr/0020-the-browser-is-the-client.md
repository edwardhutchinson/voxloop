# The browser is the client; the wrapper adds one input

VoxLoop v1 ships **one SvelteKit application, browser-first and first-class on every platform**. A Tauri wrapper is built **last**, from the same bundle, and its entire job is to register **one** additional push-to-talk input source: a native global hotkey.

This inverts the plan this ticket started with, which had operators on a Tauri desktop app and the browser as a lesser tier for admins and listeners. Two findings turned it around, and a third settled where it landed.

## Why the desktop tier stopped being the operator tier

[ADR-0006](./0006-mediasoup-carries-the-audio.md) already put the media stack in the browser via `mediasoup-client`, so the desktop tier was never carrying audio — it was carrying exactly one capability the browser lacks: **push to talk while another application is focused**, which is the normal operating posture in a control room where the operator is watching telemetry.

[#5](https://github.com/edwardhutchinson/voxloop/issues/5) found that capability holds unconditionally on Windows and macOS and **fails on Linux twice over** — `global-hotkey` has no Wayland backend, and WebKitGTK as shipped by the major distributions is built without WebRTC, so a Tauri webview on Linux has no `RTCPeerConnection` at all. A desktop-first plan therefore made the client unrunnable on one of the two machines it is developed on.

[#16](https://github.com/edwardhutchinson/voxloop/issues/16) then removed the assumption that the wrapper is a superset of the browser. WebHID needs an embedder-supplied `GetHidDelegate()`, which WebView2 appears not to provide, so a wrapped build likely **loses** headset-button PTT while gaining keyboard PTT. "Wrap it later, it only adds capability" is false.

## Why unfocused PTT comes from the hotkey, not from WebHID

[#16](https://github.com/edwardhutchinson/voxloop/issues/16) established that WebHID would very likely give the *browser* an unfocused PTT path with no wrapper at all — which briefly made browser-only real-shift-work look achievable. It was rejected on a product principle rather than a technical one.

**WebHID is a hardware requirement.** It needs a conforming device that reports a USB serial number (without one, Chromium cannot store a persistent grant and the permission dies on every unplug and restart), and Chromium's keyboard-usage-page protection means the cheap device — a footswitch emitting plain keystrokes — is precisely the one WebHID cannot read. VoxLoop is a software-only product; requiring a purchase to talk contradicts that.

So **WebHID is out of v1 entirely**, and with it the Chromium lock-in, the `WebHidAllowDevicesForUrls` enterprise policy fragment, and the Linux `udev` rule. Nothing else in the client is Chromium-specific — `mediasoup-client` ships handlers for Firefox and Safari — so the browser tier is genuinely universal. The spike that would have verified WebHID on hardware ([#17](https://github.com/edwardhutchinson/voxloop/issues/17)) is closed as out of scope rather than resolved.

The unfocused path is therefore the **native global hotkey**, and it lives in the wrapper.

## Consequences

- **Unfocused PTT is the last thing built in v1.** The web product is built and iterated first; the wrap is attempted once we are happy with it. Everything before that point must work without it — which [ADR-0021](./0021-ptt-input-is-a-level-with-liveness.md) is what makes safe.
- **The wrapper may only ever add a source to the seam.** The moment it needs a second code path anywhere else — different signalling, different audio, different UI — the one-client story has broken, and that is the signal to stop rather than to fork the shell.
- **Wrapper targets are Windows and macOS; only Windows is supported.** macOS costs approximately one line of build configuration and is the only way the hotkey path can be tested locally, since both development machines are Linux and macOS. It is a development artefact and the spec must say so, or it will be read as a supported client tier.
- **Linux is browser-only, permanently.** This is now a positive statement rather than a gap: a normal browser on Linux has full WebRTC and is a first-class VoxLoop client. The door [#5](https://github.com/edwardhutchinson/voxloop/issues/5) worried about is open by default.
- **The global hotkey steals a key from every other application.** Windows `RegisterHotKey` is exclusive, so the combination stops being delivered to the telemetry software the operator is looking at — and registration *fails* rather than warns when the combination is already taken. Conflict detection at binding time is mandatory; a silent failure here is an operator who believes they have PTT.
- **Keystroke footswitches are supported in v1 for free.** With WebHID gone they are simply keyboards, so they need no code at all. They are the only PTT peripheral v1 supports, we ship none, and headset inline buttons are unsupported.
- **If the wrapper is never built, v1 has no unfocused PTT.** The trigger that makes it mandatory rather than optional is named in the spec: the first pilot user expected to work a shift with telemetry focused.
