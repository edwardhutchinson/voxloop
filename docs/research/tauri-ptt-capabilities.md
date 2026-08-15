# Tauri push-to-talk capabilities: what holds, what doesn't

**Issue:** [#5](https://github.com/edwardhutchinson/voxloop/issues/5) · **Date of research:** 2026-08-15
**Method:** primary sources only — Tauri/wry/global-hotkey source and docs, WebKit source and WebKitGTK API
reference, distribution packaging recipes, Win32 and WebView2 reference documentation, freedesktop portal
and Wayland protocol specifications, Electron's own documentation. Secondary write-ups and blog posts were
deliberately excluded. Where a primary source is silent or ambiguous, this document says so rather than
filling the gap.

**Versions examined:** `tauri-plugin-global-shortcut` 2.3.2, `global-hotkey` 0.8.0 (latest, published
2026-05-01), wry `dev`, tauri `dev`, WebKitGTK 2.52.x, WebView2 SDK 1.0.4129.50.

---

## 1. Executive summary

The Tauri presumption **holds on Windows and macOS** and **does not hold, as-is, on Linux** — but the Linux
failure is in two independent places, and only one of them is about hotkeys.

| Requirement | Windows | Linux (X11) | Linux (Wayland) | macOS |
| --- | --- | --- | --- | --- |
| Global hotkey while unfocused | Yes | Yes | **No** | Yes |
| Press **and release** events (momentary PTT) | Yes | Yes | n/a | Yes |
| Press-detect latency | event-driven | **up to 50 ms** | n/a | event-driven |
| Release-detect latency | **up to 50 ms** | up to 50 ms | n/a | event-driven |
| `getUserMedia` in the webview | Yes | Yes (needs an explicit permission handler) | same | Yes |
| **`RTCPeerConnection` in the webview** | Yes | **No** (stock distro builds) | **No** | Yes |

Two findings dominate everything else:

1. **`global-hotkey` — the crate Tauri's plugin is built on — has no Wayland backend.** It is X11-only.
   Tauri's own documentation table says Linux support is "full" with no caveat; that table is wrong by
   omission.
2. **WebKitGTK as shipped by Debian, Ubuntu, Fedora and Arch is compiled without WebRTC.** A Tauri app on
   Linux therefore has no `RTCPeerConnection` at all. This is not a permissions problem and cannot be fixed
   by configuration — it is a build-time feature flag in the WebKit source that no mainstream distribution
   turns on.

Finding 2 is not something issue #5 asked about, but it is fatal to the obvious VoxLoop architecture (SFU +
WebRTC in a shared SvelteKit codebase) on Linux and needs to be surfaced to the stack decision.

---

## 2. Global hotkey capture while unfocused

### 2.1 How the plugin is actually built

`tauri-plugin-global-shortcut` is a thin wrapper. Its `Cargo.toml` declares exactly one platform dependency:

```toml
[target."cfg(not(any(target_os = \"android\", target_os = \"ios\")))".dependencies]
global-hotkey = { version = "0.8", features = ["serde"] }
```

— [`plugins/global-shortcut/Cargo.toml`](https://github.com/tauri-apps/plugins-workspace/blob/v2/plugins/global-shortcut/Cargo.toml)

The same file declares `linux = { level = "full", notes = "" }` under `[package.metadata.platforms.support]`,
which is what feeds the "Linux ✓ Fully supported" row on
[tauri.app/plugin/global-shortcut](https://tauri.app/plugin/global-shortcut/). That claim is inherited
metadata, not a tested statement about Wayland (see §2.4).

The plugin registers a direct callback rather than polling — events are dispatched from the OS callback
thread via `GlobalHotKeyEvent::set_event_handler`
([`plugins/global-shortcut/src/lib.rs`](https://github.com/tauri-apps/plugins-workspace/blob/v2/plugins/global-shortcut/src/lib.rs)),
so the plugin adds no latency of its own on the Rust side. The JavaScript API delivers events over a
`Channel`, i.e. across the webview IPC boundary — see §3.

The JS event carries press/release state, which is what makes momentary PTT expressible at all:

```ts
export interface ShortcutEvent {
  shortcut: string
  id: number
  state: 'Released' | 'Pressed'
}
```

— [`plugins/global-shortcut/guest-js/index.ts`](https://github.com/tauri-apps/plugins-workspace/blob/v2/plugins/global-shortcut/guest-js/index.ts)

Tauri v2's capability system gates the plugin: `global-shortcut:allow-register`,
`allow-unregister`, `allow-is-registered` etc. must be granted in the app's capabilities file. This is a
build-time author decision, not an end-user prompt
([plugin docs, Permissions section](https://tauri.app/plugin/global-shortcut/)).

### 2.2 Windows

`global-hotkey`'s Windows backend creates a message-only window and calls the Win32 `RegisterHotKey` API
with `MOD_NOREPEAT`
([`src/platform_impl/windows/mod.rs`](https://github.com/tauri-apps/global-hotkey/blob/dev/src/platform_impl/windows/mod.rs)).

`RegisterHotKey` is a system-wide hotkey facility: "When a key is pressed, the system looks for a match
against all hot keys. Upon finding a match, the system posts the `WM_HOTKEY` message to the message queue of
the window with which the hot key is associated"
([RegisterHotKey, Win32 reference](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-registerhotkey)).
Focus is irrelevant. **This is the mechanism a browser tab cannot reach, and it works.**

Limits, all from the same reference page:

- **No key-release message exists.** `WM_HOTKEY` is a press-only notification. The docs describe only the
  press path; the `MOD_NOREPEAT` example confirms it by saying the thread "will only receive another
  `WM_HOTKEY` message when the 'b' key is released and then pressed again."
- **Registration is first-come-first-served across the machine.** "Typically, `RegisterHotKey` also fails if
  the keystrokes specified for the hot key have already been registered for another hot key." `global-hotkey`
  surfaces this as `Error::AlreadyRegistered` by matching `ERROR_HOTKEY_ALREADY_REGISTERED`.
- **F12 is permanently reserved for the debugger** and must not be registered.
- **Windows-key combinations are reserved by the OS.**

Because there is no release message, `global-hotkey` synthesises one by spawning a thread that polls
`GetAsyncKeyState` — see §3 for the latency consequence. The source comment names our exact use case:

```rust
// Sleep to avoid burning a core for the whole hold duration
// (e.g. push-to-talk). 50ms keeps release latency imperceptible.
// See https://github.com/tauri-apps/global-hotkey/issues/176
std::thread::sleep(std::time::Duration::from_millis(50));
```

**Elevation and permission prompts.** The `RegisterHotKey` reference documents no privilege requirement, no
capability, and no consent prompt. It is an ordinary `user32.dll` call available to any desktop process. So:
**no UAC prompt, no elevation, no per-user grant to click through.**

**Where the docs go silent — and it matters for a control room.** The `RegisterHotKey` reference says nothing
about integrity levels. Microsoft's separate User Interface Privilege Isolation documentation establishes the
general rule that lower-privilege processes cannot send messages into higher-privilege ones, and that
bypassing it requires the `uiAccess` manifest attribute
([UIPI issues, Power Automate troubleshooting](https://learn.microsoft.com/en-us/troubleshoot/power-platform/power-automate/desktop-flows/ui-automation/uipi-issues);
[UAC / UIAccess policy reference](https://learn.microsoft.com/en-us/previous-versions/windows/it-pro/windows-server-2012-R2-and-2012/jj852245(v=ws.11))).
Whether `WM_HOTKEY` is delivered to a normal-integrity VoxLoop while an *elevated* window holds focus is
**not stated anywhere in the primary documentation I could find.** In a spacecraft ops room, telemetry and
control clients running elevated is plausible, so this is a real risk and it is on the spike list (§9).

Similarly, **AppLocker / Windows Defender Application Control can block the executable from running at all**
on a locked-down build — but that is a distribution problem (§7), not a hotkey problem. Nothing about
`RegisterHotKey` itself is privileged.

### 2.3 Linux under X11

The X11 backend calls `grab_key` on the **root window** for the requested keycode, and — because
`XGrabKey` matches modifier state exactly — repeats the grab four times to cover NumLock and CapsLock:

```rust
// XGrabKey works only with the exact state (modifiers)
// and since X11 considers NumLock, ScrollLock and CapsLock a modifier when it is ON,
// we also need to register our shortcut combined with these extra modifiers as well
fn ignored_mods() -> [ModMask; 4] { ... }
```

— [`src/platform_impl/x11/mod.rs`](https://github.com/tauri-apps/global-hotkey/blob/dev/src/platform_impl/x11/mod.rs)

It enables XKB detectable auto-repeat, and emits both `HotKeyState::Pressed` and `HotKeyState::Released`
from `Event::KeyPress` / `Event::KeyRelease`. Functionally this is the best of the three backends for PTT —
real press and real release, both from the X server.

Limits:

- **A root-window grab is exclusive per key+modifier combination.** If the desktop environment or another
  application already grabbed the combination, `grab_key` returns `Access` and the crate reports
  `Error::AlreadyRegistered`. VoxLoop must handle a failed registration as a first-class, user-visible state
  and offer rebinding — it cannot assume its key is available.
- **`ScrollLock` is not compensated for**, only NumLock and CapsLock — the comment says "ScrollLock" but
  `ignored_mods()` only covers `M2` (NumLock) and `LOCK` (CapsLock). Minor, but it means a ScrollLock-on
  keyboard can silently break the hotkey.
- Latency: the event loop polls rather than blocks — see §3.

### 2.4 Linux under Wayland — this is the hard "no"

**Wayland's protocol has no global keyboard grab, by design.** The core protocol delivers key events only to
the surface that currently holds keyboard focus:

> "The compositor must not send this event if the wl_keyboard did not have an active surface immediately
> before this event."

— [`wl_keyboard::key`, wayland.xml](https://gitlab.freedesktop.org/wayland/wayland/-/blob/main/protocol/wayland.xml)

There is no X11-style `XGrabKey` equivalent. This is not an implementation gap; it is the security model.

**`global-hotkey` has no Wayland backend.** `src/platform_impl/` contains exactly `macos/`, `windows/`,
`x11/` and `no-op.rs`. The crate README states support as "Windows, macOS, Linux (X11 Only)". The X11
backend's own connection error message says it outright:

> "Unable to open x11 connection, maybe you are not running under X11? Other window systems on Linux are not
> supported by `global-hotkey` crate"

— [`src/platform_impl/x11/mod.rs`](https://github.com/tauri-apps/global-hotkey/blob/dev/src/platform_impl/x11/mod.rs)

**The sanctioned Wayland mechanism exists and Tauri does not use it.** freedesktop's
`org.freedesktop.portal.GlobalShortcuts` (interface version 2) provides `CreateSession`, `BindShortcuts`,
`ListShortcuts`, and — critically for PTT — both `Activated` and `Deactivated` signals, i.e. press and
release
([portal documentation](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.GlobalShortcuts.html)).
Its semantics differ materially from `XGrabKey`: the application supplies a shortcut *id*, a
human-readable description and an optional `preferred_trigger`, and `BindShortcuts` "will typically result
the portal presenting a dialog showing the shortcuts and allowing users to configure the shortcuts."
**The user, not the application, owns the final binding**, and the app cannot unbind.

Portal backend availability (checked directly against each implementation's source tree):

| Desktop | GlobalShortcuts backend | Evidence |
| --- | --- | --- |
| GNOME 48+ | Shipped | [GNOME 48 release notes](https://release.gnome.org/48/): "apps can now create their own system-wide shortcuts" |
| KDE Plasma | Shipped | `src/globalshortcuts.cpp`, `src/GlobalShortcutsDialog.qml` in [xdg-desktop-portal-kde](https://invent.kde.org/plasma/xdg-desktop-portal-kde) |
| Hyprland | Shipped | `src/portals/GlobalShortcuts.cpp` in [xdg-desktop-portal-hyprland](https://github.com/hyprwm/xdg-desktop-portal-hyprland) |
| wlroots (Sway etc.) | **Not implemented** | [xdg-desktop-portal-wlr#240](https://github.com/emersion/xdg-desktop-portal-wlr/issues/240) still open |

**Tauri's Wayland support is an open, unmerged PR.**
[global-hotkey#28](https://github.com/tauri-apps/global-hotkey/issues/28) ("Global Shortcut support on
Wayland") has been open since 2023. [PR #162](https://github.com/tauri-apps/global-hotkey/pull/162)
implements it via the portal, is **open, not merged**, last touched 2026-06-19, and deliberately exposes a
*separate* API surface (`GlobalHotKeyManager::wl_register_all()`) because the portal's model is too different
to unify. A Tauri maintainer's position on the issue: "I have nobody available rn that i can assign Linux
related tasks to so this has to rely on community contributions."
[plugins-workspace#3267](https://github.com/tauri-apps/plugins-workspace/issues/3267)
("`tauri_plugin_global_shortcut` callback doesn't work in wayland") is likewise open, with a maintainer
pointing at the same unmerged PR.

**What happens in practice under a Wayland session?** The crate will attempt an X11 connection. If XWayland
is running (the usual case) the connection *succeeds*, and `grab_key` on the XWayland root window will
*succeed too* — but XWayland only receives key events the compositor routes to it, and the compositor only
routes keys to the focused surface. So the plausible failure mode is **silent**: registration reports
success, and the hotkey simply never fires when VoxLoop is unfocused. I could not find a primary source that
states XWayland's grab behaviour explicitly, so **treat "registers successfully but never fires" as the
expected-but-unverified outcome** and confirm it in a spike (§9). What *is* certain from primary sources is
that native Wayland gives no unfocused key delivery and that Tauri does not speak the portal.

**This matters more each year.** Ubuntu 25.10 ships webkit2gtk 2.52.3 on a GNOME desktop where Wayland is the
default session; contributors on global-hotkey#28 report Ubuntu 25.10 dropping X11 and KDE Plasma 6.1
requiring manual X11 installation. I could not retrieve the Ubuntu release notes to confirm the X11-removal
claim directly (the Discourse page 404'd), so **treat the specific distro claims as unverified** — but the
direction of travel is not in doubt.

**Mitigations, in order of cost:**

1. Force an X11 session (`GDK_BACKEND=x11` / choosing "GNOME on Xorg" at login). Viable for a controlled
   ops-room fleet today; a shrinking option.
2. Talk to the portal directly from VoxLoop's Rust side using
   [`ashpd`](https://docs.rs/ashpd/latest/ashpd/desktop/global_shortcuts/index.html) (0.13.13, July 2026,
   has a `desktop::global_shortcuts` module). This is exactly what the global-hotkey maintainers suggest
   applications do. It means writing and owning the Wayland path ourselves, plus UI for the fact that the
   *user* picks the key, not us.
3. Read `/dev/input` via evdev directly. Bypasses the display server entirely, works on both X11 and
   Wayland, and needs the process to be in the `input` group — a privileged read of all keystrokes on the
   machine, which is a security conversation with the customer, not a technical one.

### 2.5 macOS (bonus)

The macOS backend uses Carbon's `RegisterEventHotKey` and installs a handler for **both**
`kEventHotKeyPressed` and `kEventHotKeyReleased`
([`src/platform_impl/macos/mod.rs`](https://github.com/tauri-apps/global-hotkey/blob/dev/src/platform_impl/macos/mod.rs)).
Fully event-driven, no polling, real press and release. `RegisterEventHotKey` is not an event-tap API, so it
does not require Accessibility/Input Monitoring consent. The crate requires the manager to be created on the
main thread with a running event loop (README). This is the cleanest of the three implementations.

---

## 3. Latency and reliability

The ticket sets the bar at "a PTT key that drops the first 200 ms of speech is an operational failure."
Here is what the source actually does, per platform. These are code-derived facts, not benchmarks — no
primary source publishes measured latency figures for any of these paths.

| Platform | Press → event | Release → event | Mechanism |
| --- | --- | --- | --- |
| Windows | Event-driven (`WM_HOTKEY` posted by the system) | **0–50 ms** | `GetAsyncKeyState` polled on a spawned thread with `sleep(50ms)` |
| Linux/X11 | **0–50 ms** | **0–50 ms** | `conn.poll_for_event()` in a loop ending `std::thread::sleep(Duration::from_millis(50))` |
| macOS | Event-driven | Event-driven | Carbon event handler for pressed and released |

The X11 event loop is a polling loop, not a blocking wait:

```rust
loop {
    while let Ok(Some(event)) = conn.poll_for_event() { /* ... */ }
    if let Ok(msg) = thread_rx.try_recv() { /* ... */ }
    std::thread::sleep(std::time::Duration::from_millis(50));
}
```

**Interpretation.** Nothing here approaches 200 ms of clipped speech from the hotkey layer alone. Worst case
is ~50 ms on Linux press-detect, and human reaction between pressing a key and starting to speak comfortably
exceeds that. Late *release* detection (Windows and Linux, up to 50 ms) is operationally benign: it leaves
the mic open slightly longer, it does not truncate speech.

**But the hotkey layer is not the whole latency budget.** Two further hops the primary sources do not
quantify:

- The plugin's JS path pushes events across the webview IPC `Channel`. If PTT gating is implemented in
  JavaScript, that IPC hop is on the critical path and its latency is undocumented. **Gate the audio in
  Rust, via the plugin's Rust `on_shortcut` handler, not in the webview** — then the IPC hop only carries UI
  state updates and is off the critical path.
- Getting from "PTT pressed" to "packets on the wire" depends on the audio pipeline (capture buffer size,
  encoder frame size, SFU forwarding), which is outside this ticket. If the design keeps the mic track open
  and merely toggles transmission, the hotkey latency is the only PTT latency; if it calls `getUserMedia` on
  each press, device-open time (unbounded, tens to hundreds of ms) dominates and 200 ms *is* at risk.

**Reliability caveats already covered:** exclusive registration means another app can take the key
(Windows: `ERROR_HOTKEY_ALREADY_REGISTERED`; X11: `BadAccess` → `AlreadyRegistered`). Registration failure
must be a visible, actionable state in the UI — consistent with the wayfinder constraint that the UI must
never misrepresent emission state.

---

## 4. Microphone capture from the webview

### 4.1 Secure context

`getUserMedia` requires a secure context. Both Tauri origins qualify:

- **Windows**: the app is served from `http://tauri.localhost`
  ([`crates/tauri-utils/src/config.rs`](https://github.com/tauri-apps/tauri/blob/dev/crates/tauri-utils/src/config.rs)
  documents `access-control-allow-origin: http://tauri.localhost`). The W3C Secure Contexts algorithm returns
  "Potentially Trustworthy" if the host "ends with `.localhost`"
  ([W3C Secure Contexts](https://w3c.github.io/webappsec-secure-contexts/)).
- **Linux**: wry explicitly registers its custom scheme as secure —
  `security_manager().register_uri_scheme_as_secure(name)` with the comment `// Enable secure context`
  ([`src/webkitgtk/web_context.rs`](https://github.com/tauri-apps/wry/blob/dev/src/webkitgtk/web_context.rs)).

So no secure-context gap versus a browser.

### 4.2 The permission model

wry exposes a cross-platform permission handler with `PermissionKind::Microphone` and a tri-state response
([`src/permissions.rs`](https://github.com/tauri-apps/wry/blob/dev/src/permissions.rs)). Tauri surfaces it as
`WebviewBuilder::on_permission_request(|webview, kind| -> PermissionResponse)`
([`crates/tauri/src/webview/mod.rs`](https://github.com/tauri-apps/tauri/blob/dev/crates/tauri/src/webview/mod.rs)),
wired through in
[`crates/tauri-runtime-wry/src/lib.rs`](https://github.com/tauri-apps/tauri/blob/dev/crates/tauri-runtime-wry/src/lib.rs).

The defaults differ per platform, and this is the load-bearing detail. From wry's own doc comments on
`PermissionResponse::Default`:

> - **Windows / macOS / Android**: The default behavior is to continue the platform or browser permission flow.
> - **Linux**: The default behavior is `Self::Deny`

**Windows.** wry maps to WebView2's `PermissionRequested` event. If the app does not set a state, the
documented default is `CoreWebView2PermissionState.Default` = "Specifies that the default browser behavior is
used, which normally prompt users for decision"
([CoreWebView2PermissionState](https://learn.microsoft.com/en-us/dotnet/api/microsoft.web.webview2.core.corewebview2permissionstate)).
So out of the box the operator sees a WebView2 mic prompt. VoxLoop should set `PermissionResponse::Allow` for
`Microphone` explicitly — the app *is* the microphone application, and an unexpected prompt at shift start is
exactly the cognitive load the wayfinder map wants removed.

**Linux.** wry only connects WebKitGTK's `permission-request` signal *if a permission handler was supplied*
([`src/webkitgtk/mod.rs`](https://github.com/tauri-apps/wry/blob/dev/src/webkitgtk/mod.rs), the block guarded
by `if let Some(permission_handler) = attributes.permission_handler.take()`). Without one, and given that
Linux's documented default is Deny, `getUserMedia` fails on Linux. **On Linux, calling
`on_permission_request` and returning `Allow` for `Microphone` is mandatory, not optional.** This is the
mechanism behind the long tail of "NotAllowedError on Ubuntu" reports in the Tauri tracker (e.g.
[tauri#8346](https://github.com/tauri-apps/tauri/issues/8346)); note that
[tauri#15277](https://github.com/tauri-apps/tauri/issues/15277) was closed by its reporter as a local
integration problem rather than a confirmed upstream bug, so I am not citing it as evidence of a defect.

**Windows OS-level gate.** Beyond WebView2, Windows has a system privacy toggle. Microsoft's documentation:
"Desktop apps cannot be individually toggled, but access for those apps can be controlled using **Let desktop
apps access your microphone**", and "Turning the setting on or off will impact all apps listed under this
setting"
([Windows camera, microphone, and privacy](https://support.microsoft.com/en-us/windows/privacy/windows-camera-microphone-and-privacy)).
On a managed fleet this is a single Group Policy / MDM switch that either works for every desktop app or none
— it is not VoxLoop-specific, and any existing softphone or Teams deployment proves it is already on. Worth
naming in the deployment checklist regardless.

**macOS.** Requires `NSMicrophoneUsageDescription` in the bundle `Info.plist`; without it the process is
terminated on first mic access. This is Apple's standard TCC behaviour and has been the recommendation on
[wry#85](https://github.com/tauri-apps/wry/issues/85) since 2020.

### 4.3 The gap that actually bites: WebRTC on Linux

`getUserMedia` working is not the same as WebRTC working, and on Linux they come apart.

WebKit's GTK port build configuration:

```cmake
WEBKIT_OPTION_DEFAULT_PORT_VALUE(ENABLE_MEDIA_STREAM PRIVATE ON)
WEBKIT_OPTION_DEFAULT_PORT_VALUE(ENABLE_WEB_RTC     PRIVATE ${ENABLE_EXPERIMENTAL_FEATURES})
```

— [`Source/cmake/OptionsGTK.cmake`](https://github.com/WebKit/WebKit/blob/main/Source/cmake/OptionsGTK.cmake),
verified identical in the 2.52.5 tarball Debian ships
([sources.debian.org](https://sources.debian.org/src/webkit2gtk/2.52.5-1/Source/cmake/OptionsGTK.cmake/))

and `ENABLE_EXPERIMENTAL_FEATURES` itself:

```cmake
option(ENABLE_EXPERIMENTAL_FEATURES "Enable experimental features" OFF)
```

— [`Source/cmake/WebKitFeatures.cmake`](https://github.com/WebKit/WebKit/blob/main/Source/cmake/WebKitFeatures.cmake)

So `MediaStream` (hence `getUserMedia`) is on by default; **`WEB_RTC` is off unless the packager opts into
experimental features.** I checked whether the mainstream packagers do:

| Distribution | Enables `ENABLE_WEB_RTC` / `ENABLE_EXPERIMENTAL_FEATURES`? | Source |
| --- | --- | --- |
| Debian (webkit2gtk 2.52.5-1) | **No** | [`debian/rules`](https://sources.debian.org/src/webkit2gtk/2.52.5-1/debian/rules/) |
| Fedora (rawhide) | **No** | [`webkitgtk.spec`](https://src.fedoraproject.org/rpms/webkitgtk/blob/rawhide/f/webkitgtk.spec) — the `%cmake` invocations pass neither |
| Arch (webkit2gtk-4.1) | **No** | [`PKGBUILD`](https://gitlab.archlinux.org/archlinux/packaging/packages/webkit2gtk-4.1/-/blob/main/PKGBUILD) |
| Ubuntu 25.10 | Inherits Debian packaging; ships 2.52.3-0ubuntu0.25.10.1 ([Launchpad](https://launchpad.net/ubuntu/+source/webkit2gtk)) | Not separately verified — flagged in §9 |

WebKitGTK's own API reference records the runtime property's default as FALSE and notes that enabling it also
enables media-stream
([`WebKitSettings:enable-webrtc`](https://webkitgtk.org/reference/webkitgtk/stable/property.Settings.enable-webrtc.html),
since 2.38). But when the library is compiled without `ENABLE(WEB_RTC)`, the property is hardcoded FALSE in
`WebKitSettings.cpp` and setting it at runtime does nothing —
[`Source/WebKit/UIProcess/API/glib/WebKitSettings.cpp`](https://github.com/WebKit/WebKit/blob/main/Source/WebKit/UIProcess/API/glib/WebKitSettings.cpp)
falls through to a literal `FALSE` in the `#else` branch. wry does not set either property.

Tauri maintainers state the position plainly on
[tauri#13143](https://github.com/tauri-apps/tauri/issues/13143): "webkitgtk does not yet support webrtc", and,
on whether a custom build helps, "there's a reason webrtc is behind an experimental buildtime flag that's
disabled by default, it simply doesn't work properly yet." The maintainer adds that basic mic/camera capture
does work on some distros, "but that doesn't include much of actual webrtc stuff like peer connections."
[wry#85](https://github.com/tauri-apps/wry/issues/85) ("WebRTC support on Linux") remains open.

**Consequence for VoxLoop.** If the audio transport is browser WebRTC against a self-hosted SFU — the
architecture the wayfinder map's constraints point at — that design **does not run in a Tauri webview on
Linux**. Windows (WebView2 = Chromium) and macOS (WKWebView) are unaffected. This is the single largest
finding in this document and it is not a hotkey problem.

Options, none free:

1. **Windows-only operator client.** If every operator console is Windows, the problem evaporates. This is a
   customer-fact question, not a technical one, and it should be asked before anything else.
2. **Move audio out of the webview into Rust.** Capture with a native audio crate and run the WebRTC
   endpoint natively (`webrtc-rs`, `str0m`), with the webview reduced to UI. This makes the desktop client
   materially different from the browser client and partly undoes "two client tiers, one codebase" — but it
   also removes the webview from the real-time path entirely, which for a critical-path voice application is
   arguably the right shape anyway.
3. **Ship our own WebKitGTK.** Rejected on the maintainers' own evidence that the experimental backend
   "doesn't work properly yet", and it would mean owning a browser engine build in a self-hosted, possibly
   air-gapped deployment.
4. **Electron on Linux.** Chromium, full WebRTC, no question. See §6.

---

## 5. Hardware peripherals: footswitches and headset PTT buttons

The answer splits cleanly along how the device enumerates, and the platform primary sources settle it.

**Devices that enumerate as HID keyboards — catchable, no integration needed.** Windows opens usage page
`0x0001`, usages `0x0006`–`0x0007` (Keyboard) **Exclusive** for system use
([Top-Level Collections Opened by Windows for System Use](https://learn.microsoft.com/en-us/windows-hardware/drivers/hid/top-level-collections-opened-by-windows-for-system-use)).
The system keyboard stack owns them and injects ordinary keystrokes; `RegisterHotKey` sees them like any
other key. On Linux the kernel's `hid-input.c` maps the keyboard usage page to standard `KEY_*` codes, which
reach X11 as normal keycodes. **A programmable USB footswitch configured to send a keystroke works with zero
extra code.** This is the recommended procurement path.

**Headset call-control / PTT buttons — generally *not* catchable as keys.** These live on the HID Telephony
Device page (`0x000B`), which is conspicuously **absent** from Windows' system top-level-collection table
above: Windows does not claim it and does not synthesise keyboard input from it. On Linux, the kernel maps
almost nothing from that page:

```c
case HID_UP_TELEPHONY:
    switch (usage->hid & HID_USAGE) {
    case 0x2f: map_key_clear(KEY_MICMUTE);  break;
    case 0xb0: map_key_clear(KEY_NUMERIC_0); break;
    /* ... numeric keypad usages 0xb1–0xbf ... */
    default: goto ignore;
    }
```

— [`drivers/hid/hid-input.c`](https://github.com/torvalds/linux/blob/master/drivers/hid/hid-input.c)

Only Phone Mute (`0x2F`) and the numeric usages are mapped; Hook Switch and the rest hit `goto ignore`. So a
headset's off-hook/call button produces **no key event on either platform**. Catching it requires reading the
HID collection directly (Raw Input / `HidD_*` on Windows, hidraw or a udev-permitted evdev node on Linux), or
a vendor SDK.

**Consumer control page (`0x000C`, usage `0x0001`) is opened Shared by Windows** — devices that map their
button to a consumer usage (e.g. media keys) are visible to the system but are not `RegisterHotKey`
territory either; `global-hotkey`'s `key_to_vk` covers virtual-key codes, and mouse buttons are explicitly
unsupported ([global-hotkey#102](https://github.com/tauri-apps/global-hotkey/issues/102), closed).

**Where the primary sources are silent.** Which *specific* footswitch and headset models present as HID
keyboards versus telephony devices is documented only by the device vendors, not by any platform spec. I did
not verify individual products. The rule to hand procurement is: **"must be configurable to send a plain
keyboard keystroke"** — that is the only property that makes a peripheral work with the global-shortcut path
on every platform without driver work.

---

## 6. Packaging and distribution on managed corporate machines

**Formats** ([tauri.app/distribute](https://tauri.app/distribute/)):

- Windows: MSI (WiX v3, buildable only on Windows) and NSIS `.exe` (cross-compilable), plus Microsoft Store.
- Linux: AppImage, `.deb`, `.rpm`, Snap, Flatpak, AUR.
- macOS: `.app` bundle, DMG, App Store.

**Windows install modes** ([Windows Installer docs](https://tauri.app/distribute/windows-installer/)): NSIS
supports per-user (default, installs to `%LOCALAPPDATA%`, **no administrator privileges required**),
per-machine (Program Files, requires elevation), or both. **Per-user install is the pilot's friend** — it is
deployable on a locked-down machine without a desktop-support ticket.

**WebView2 runtime.** Five strategies: `downloadBootstrapper` (default, ~0 MB, needs internet at install
time), `embedBootstrapper` (~1.8 MB), `offlineInstaller` (~127 MB, **the right choice for an air-gapped
customer network**), `fixedVersionRuntime` (~180 MB, pins the exact engine version — attractive for a
change-controlled ops environment where a silent Evergreen update must not be able to break the voice loop),
or `skip`. Windows 11 and Windows 10 from April 2018 onward include WebView2.

**Code signing.** Tauri's own documentation
([Windows signing](https://tauri.app/distribute/sign/windows/)) is unusually candid: signing "is not required
to execute your application on Windows, as long as your end user is okay with ignoring the SmartScreen
warning." An EV certificate gets immediate SmartScreen reputation; an OV certificate still triggers warnings
until reputation accrues ("It might take some time until your certificate builds enough reputation").

**So: can an unsigned build be deployed for a pilot?** From the documentation, yes — SmartScreen shows a
warning that a user can click through, and the warning is a *download* reputation check, not an execution
ban. **But** that is Tauri's answer, not the customer's IT policy's answer. AppLocker or WDAC in enforcement
mode can block unsigned binaries outright, and no Tauri documentation can tell us whether this customer runs
it. Practical guidance: ask for the application-control policy before assuming, and if a signed build is
needed, note that **OV is sufficient for a pilot delivered by IT (sideloaded, not browser-downloaded)** —
SmartScreen's download reputation heuristic is largely bypassed when IT deploys the installer through their
own channel. That last inference is mine, not documented.

**Linux.** No equivalent signing gate. `.deb`/`.rpm` for a managed fleet, AppImage for zero-install pilots.
Tauri's Linux build dependency is `libwebkit2gtk-4.1` ([prerequisites](https://tauri.app/start/prerequisites/)),
which constrains which distributions can even run the client — anything older than roughly Ubuntu 22.04 /
Debian 12 / Fedora 36 lacks the 4.1 API and is out. Worth confirming against the customer's actual Linux
estate, if they have one.

---

## 7. Auto-update for a self-hosted, possibly air-gapped deployment

From the [updater plugin documentation](https://tauri.app/plugin/updater/):

- **Signing is mandatory and cannot be disabled.** A minisign keypair is generated via `tauri signer
  generate`; the public key is embedded in `tauri.conf.json`, the private key signs artifacts at build time.
  "If you lose this key you will NOT be able to publish new updates to the users that have the app already
  installed." **Key custody is a v1 operational requirement, not a later concern.**
- Two update sources: a **static JSON manifest** at a URL, or a **dynamic endpoint** that returns metadata or
  HTTP 204. Endpoints support `{{current_version}}`, `{{target}}`, `{{arch}}` substitution, plus runtime
  proxy configuration and custom auth headers.
- **TLS is enforced in production**, with `dangerousInsecureTransportProtocol` as an explicit escape hatch
  for non-HTTPS endpoints.

**Fit for VoxLoop.** Good. The static-JSON mode is trivially self-hostable — the VoxLoop server itself can
serve the manifest and the artifacts, no external dependency, which satisfies the air-gap constraint
directly. The minisign scheme means the update channel is authenticated independently of transport, so an
internal HTTP-only endpoint is defensible if the customer's PKI makes internal TLS awkward.

**Where the docs are silent.** Nothing in the updater documentation addresses a fully offline install
(sneakernet USB) path, staged/phased rollout, or rollback to a previous version. For a critical-path
application in a control room, "how do we back out a bad release at 3 a.m." is a real question the plugin
does not answer, and the answer is probably "re-run the previous installer", which needs testing.

---

## 8. What Tauri makes harder than Electron — and where it is better

Being honest about this, since the ticket asks.

**Where Electron is clearly better for VoxLoop:**

1. **WebRTC on Linux.** Electron bundles Chromium. Full `RTCPeerConnection`, everywhere, no distro
   dependency. This is not a close call — it is the decisive difference (§4.3).
2. **Wayland global shortcuts, today.** Electron's own documentation says: "it is also possible to use
   Chromium's `GlobalShortcutsPortal` implementation, which allows apps to bind global shortcuts when running
   within a Wayland session", enabled with
   `app.commandLine.appendSwitch('enable-features', 'GlobalShortcutsPortal')`
   ([Electron globalShortcut](https://www.electronjs.org/docs/latest/api/global-shortcut)). Tauri's
   equivalent is an unmerged PR.
3. **Engine consistency.** One Chromium version across Windows, Linux and macOS; no "works on the dev's
   Fedora box, fails on the operator's Ubuntu" class of bug. For a codebase shared with a browser admin
   console, testing surface collapses.

**Where Tauri is clearly better for VoxLoop:**

1. **Press *and* release events.** This is the sleeper. Electron's `globalShortcut.register(accelerator,
   callback)` fires "when the registered shortcut is pressed" — the API has **no key-up event at all**. There
   is no `Released` state anywhere in the documented surface. **Electron's `globalShortcut` cannot express
   momentary push-to-talk without a native module**; Tauri's plugin exposes
   `state: 'Pressed' | 'Released'` as a first-class field. For our single load-bearing requirement, Tauri's
   API is the better-shaped one. (Latched mode works fine on either.)
2. Binary size, memory footprint, and no bundled browser to patch — relevant but not decisive here.

**Where they are the same:** exclusive registration and silent contention (Electron: "when the accelerator is
already taken by other applications, this call will silently fail" — note *silently*, which is worse than
Tauri's typed `AlreadyRegistered` error); macOS media-key shortcuts needing trusted-accessibility
authorization; Windows behaving well.

**Net assessment.** If the operator fleet is Windows-only, Tauri wins on the PTT semantics and costs nothing.
If Linux operator consoles are in scope, Electron's advantages are structural and Tauri's are cosmetic —
unless VoxLoop moves audio out of the webview into Rust, in which case Tauri regains the lead decisively,
because a native Rust audio path is natural in Tauri and awkward in Electron.

---

## 9. Verdict

**Does the Tauri presumption hold?**

**Yes on Windows, unconditionally.** Global hotkeys work unfocused via `RegisterHotKey` with no elevation and
no consent prompt; press and release are both available; press latency is event-driven and release latency is
bounded at 50 ms; `getUserMedia` and full WebRTC work in WebView2; per-user installs need no admin rights;
unsigned pilot builds are deployable subject to the customer's application-control policy. If VoxLoop's
operator consoles are Windows, ship Tauri.

**Yes on macOS.** The cleanest hotkey implementation of the three, full WebRTC in WKWebView, standard TCC
plumbing.

**No on Linux, as-is — for two independent reasons.** Under Wayland there is no global hotkey capability at
all in released Tauri, and the sanctioned portal path is an unmerged PR. Independently, and regardless of
display server, the WebKitGTK that Debian, Fedora and Arch actually ship is compiled without WebRTC, so a
browser-WebRTC audio design does not run there.

**Conditions under which it holds on Linux:**

- Operators run an X11 session (viable now on a controlled fleet, shrinking); **and**
- Audio transport does not depend on `RTCPeerConnection` inside the webview — i.e. the WebRTC endpoint lives
  in Rust and the webview is UI only; **and**
- `on_permission_request` explicitly returns `Allow` for `PermissionKind::Microphone`, without which Linux
  denies by default.

Meeting those three conditions is a real architectural commitment, not a configuration tweak. If they cannot
be met and Linux consoles are in scope, **Electron is the better fit and the evidence says so plainly**.

**Recommended next question for the wayfinder, ahead of any spike:** *are the operator consoles Windows?* A
"yes" retires most of this document.

### What cannot be settled from documentation — the spike list

Each of these is a specific, cheap experiment; none can be answered by reading.

1. **XWayland grab behaviour.** Register a hotkey under a GNOME/KDE Wayland session with XWayland present.
   Does `grab_key` succeed and then never fire? Does registration fail outright? The failure *mode* matters:
   silent success is far more dangerous operationally than a clean error, and it changes what the UI must
   detect and show.
2. **Windows elevated-focus delivery.** With VoxLoop at normal integrity, focus an elevated window and press
   the PTT key. Is `WM_HOTKEY` delivered? The Win32 reference is silent on integrity levels and the answer
   determines whether VoxLoop must ship with a `uiAccess` manifest (which itself requires signing and
   installation to a protected path).
3. **End-to-end PTT latency.** Measure keypress → first RTP packet, not just keypress → event. The 50 ms
   polling figures are code-derived; the audio pipeline contribution is unmeasured and is where a 200 ms
   budget would actually be spent.
4. **Ubuntu's WebKitGTK build flags.** Debian, Fedora and Arch are confirmed to build without WebRTC; Ubuntu
   inherits Debian's packaging but I did not read Ubuntu's `debian/rules` directly. Confirm before treating
   the Linux WebRTC verdict as universal.
5. **Real peripherals.** Put the candidate footswitch and headset on a Windows box and a Linux box and see
   whether they generate keycodes. The specs say "it depends how the vendor built it" and only the hardware
   can answer.
6. **Linux mic capture without WebRTC.** Confirm `getUserMedia(audio)` genuinely works on stock Ubuntu
   WebKitGTK with an explicit `Allow` handler. `ENABLE_MEDIA_STREAM` is ON by default in the build config, but
   capture also depends on the GStreamer plugins present at runtime, which no primary source enumerates.
7. **Updater rollback.** Verify that re-running a previous installer cleanly downgrades a deployed client.
