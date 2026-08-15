# WebHID and unfocused push-to-talk

Research note for [issue #16](https://github.com/edwardhutchinson/voxloop/issues/16), against the constraints on
the wayfinder map ([issue #1](https://github.com/edwardhutchinson/voxloop/issues/1)). It resolves a contradiction
between the Tauri PTT research ([issue #5](https://github.com/edwardhutchinson/voxloop/issues/5)) and the openvocs
evaluation ([issue #15](https://github.com/edwardhutchinson/voxloop/issues/15)), and blocks the client-tier
decision ([issue #9](https://github.com/edwardhutchinson/voxloop/issues/9)).

**Researched:** 2026-08-15, against the WebHID spec text (WICG Draft Community Group Report, 13 September 2024),
the Chromium source tree at [`d8a3ba21`](https://chromium.googlesource.com/chromium/src/+/d8a3ba218dabe95a746bd44f4d5747b2f3d72725)
(`main`), the WICG blocklist file, the Linux kernel `hidraw` driver, systemd's shipped udev rules, Mozilla's and
WebKit's own standards-position repositories, Mozilla Bugzilla, the WebView2 API reference and Tauri's own issue
tracker, Electron's source and documentation as an open-source embedder comparison, and a clone of
`openvocs/openvocs` at `main`.
Every claim below cites the file, line and revision it came from. Where a source is silent, this note says so
rather than filling the gap.

---

## 0. Verdict

**Yes — nothing in the spec or in Chromium suppresses `inputreport` when the page is unfocused, backgrounded or
hidden, and I traced every layer of the delivery path to establish that. But no primary source *states* it, and
no test asserts it, so this is a source-code deduction rather than a documented guarantee, and it needs one
hands-on spike before it can carry an architecture decision.**

Four findings drive the verdict:

1. **The spec has no concept of focus or visibility at all.** The words *focus*, *visible*, *hidden*,
   *background*, *foreground*, *frozen*, *lifecycle* and *fully active* do not occur anywhere in the WebHID
   specification. The input-report algorithm is unconditional: on receipt it queues a task and fires the event,
   with the *only* precondition being the blocklist check ([§1.1](#11-what-the-spec-requires)).

2. **Chromium's delivery path contains no focus or visibility check at any of its four layers** — the platform
   read (`hid_connection_win.cc` / `hid_connection_linux.cc`), the device-service filter
   (`services/device/hid/hid_connection.cc:230`), the browser-process broker
   (`content/browser/hid/hid_service.cc`), and the renderer-side event dispatch
   (`third_party/blink/renderer/modules/hid/hid_device.cc:230`). The last of these dispatches the event
   synchronously with no state test whatsoever ([§1.2](#12-what-chromium-implements)).

3. **The one mechanism that *could* suppress it — page-lifecycle freezing — is triple-defended.** The mojo
   receiver is bound to `TaskType::kMiscPlatformAPI`, which maps to `DeferrableTaskQueueTraits`: **not
   throttleable**, **runs in background**, freezable only when the page is frozen. Blink's background-tab
   freezing is **disabled by default on desktop**, and Chrome's browser-side `FreezingPolicy` names
   `CannotFreezeReason::kConnectedToHidDevice` — *"connected to HID device"* — as an explicit reason a page
   **cannot** be frozen. Using WebHID also disables the back/forward cache for the frame
   ([§1.3](#13-the-scheduler-the-only-place-it-could-break)).

4. **The scenario VoxLoop actually cares about does not even register as backgrounded.** An operator watching a
   telemetry app with the VoxLoop window still on screen is `Visibility::VISIBLE` — Chromium's page visibility is
   about pixels on screen, not keyboard focus. Focus is not an input to any of this. The WICG blocklist's own
   comment says the quiet part out loud: HID access to keyboards *"allows web pages to subvert the operating
   system's focus model"* — which is precisely the property VoxLoop wants
   ([blocklist.txt](https://github.com/WICG/webhid/blob/main/blocklist.txt)).

**But three things this does not settle, and they are load-bearing:**

- **The keyboard-class footswitch is the one device WebHID cannot read.** Issue #5 found that footswitches
  emitting plain keystrokes "work for free". Those are exactly the devices Chromium's protected-usage rule
  excludes — the Keyboard usage page (`0x07`) and Generic Desktop Keyboard/Keypad usages are *always* protected,
  and a device whose reports are all protected is not even listed by `getDevices()`. WebHID and keystroke-emitting
  footswitches are **mutually exclusive paths**, not complementary ones ([§4](#4-device-reality)).
- **Permission persistence depends on the device having a USB serial number string.** Without one, Chromium
  grants only an *ephemeral* permission, revoked on unplug and on browser restart — a per-shift re-click, which
  is the control-room failure mode #16 warned about. There is a clean fix (enterprise policy), but it must be
  deployed deliberately ([§2](#2-the-permission-model)).
- **This is Chromium-only, permanently.** Mozilla's recorded position is **negative** ("harmful"); WebKit has
  taken no position and has no implementation. On Linux, every deployment must ship a udev rule
  ([§3](#3-browser-reach)).
- **And "Chromium" may not include Tauri's WebView2.** Whether WebHID actually functions inside a Tauri webview
  on Windows is **not established** — `navigator.hid` is exposed, but nothing I can read shows a working
  `requestDevice()`, the failure mode is a silent empty array, and the persistence and enterprise-pre-grant
  mechanisms that make it deployable are Chrome-*browser*-layer code a wrapper does not inherit. A Tauri build
  could plausibly *lose* a peripheral capability the plain browser has ([§5](#5-webhid-inside-tauris-webview2-on-windows)).

**What would settle it properly**: one spike, ~30 minutes, with a real device — open a HID PTT device, log
`inputreport` with timestamps, then (a) click into another application with the VoxLoop window still visible,
(b) fully cover the window, (c) minimise it, (d) leave it minimised for 15 minutes and press again. Then ten minutes
more: the same page inside a minimal Tauri build on Windows, to answer §5. That is Ed's to run. The source says
all four states will work; nothing but a device can prove it, and nothing but a build can answer §5 at all.

---

## 1. Focus and visibility gating

### 1.1 What the spec requires

The WebHID API is a **WICG Draft Community Group Report** dated 13 September 2024, edited by Matt Reynolds
(Google) — *"not a W3C Standard nor is it on the W3C Standards Track"*
([wicg.github.io/webhid](https://wicg.github.io/webhid/)). That status matters for §3 but not for this question.

I converted the full spec text and searched it. The strings `focus`, `visib`, `hidden`, `background`,
`foreground`, `active document`, `fully active`, `freez`, `lifecycle` and `bfcache` occur **zero times**. There is
no visibility condition to interpret, because the concept is absent.

The normative algorithm ([§7 HIDDevice interface](https://wicg.github.io/webhid/#hiddevice-interface)) is:

> When an input report is received from a device, run the following steps:
> 1. If the input report is a **blocked report**, abort these steps.
> 2. Let *reportId* be the report ID for this report, or 0 if the HID interface does not use report IDs.
> 3. Let *data* be a `DataView` created over the byte sequence representing the input report. […]
> 4. **Queue a global task** on the relevant global object of *this* using the **HID device task source** to
>    fire an event named `inputreport` at *device* […]

One precondition, and it is the blocklist. "Queue a global task" is the ordinary task-queueing primitive; nothing
in the spec gates that task source on document state.

The spec's [Security and Privacy Considerations](https://wicg.github.io/webhid/#x3-security-and-privacy-considerations)
are explicitly **non-normative** and list four mitigations — chooser-based consent, a device blocklist, blocking
keyboard-like collections by usage, and a persistent in-use indicator. Focus is not among them. The section's
closest statement is under *Attacking the Host*:

> A major concern is whether the device can be used to generate **trusted input events**. These events serve as a
> proxy for user intent and can be used to access more powerful web platform features.

That is the reason keyboards are blocked (§4). It is not a focus requirement on non-keyboard devices.

**I also checked the spec's issue tracker.** `WICG/webhid` has no open or closed issue mentioning focus,
visibility, unfocused delivery, throttling or freezing. The question has never been raised there.

**Conclusion:** the spec permits delivery to an unfocused page and says nothing that would forbid it. It also
does not *require* delivery — a UA that chose to gate on focus would not be violating anything written down. This
is a permissive silence, not a guarantee.

### 1.2 What Chromium implements

The delivery path has four layers. I read all four.

**(a) Platform read — outside the renderer entirely.**
On Windows the device is opened with `CreateFile` and read with overlapped `ReadFile`
([`hid_service_win.cc:709-717`](https://chromium.googlesource.com/chromium/src/+/main/services/device/hid/hid_service_win.cc),
[`hid_connection_win.cc:268`](https://chromium.googlesource.com/chromium/src/+/main/services/device/hid/hid_connection_win.cc)).
On Linux the `/dev/hidraw*` node is opened directly and set non-blocking
([`hid_service_linux.cc:452-500`](https://chromium.googlesource.com/chromium/src/+/main/services/device/hid/hid_service_linux.cc)).
Neither file contains the string `focus` or `foreground`. These are kernel file handles held by the browser /
device-service process. They have no relationship to any window.

**(b) Device-service filter.** `HidConnection::ProcessInputReport` applies exactly one test before forwarding:

```cpp
uint8_t report_id = buffer->data()[0];
if (IsReportProtected(report_id, HidReportType::kInput)) {
  return;
}
if (client_) {
  client_->OnInputReport(buffer, size);
}
```
— [`services/device/hid/hid_connection.cc:229-236`](https://chromium.googlesource.com/chromium/src/+/main/services/device/hid/hid_connection.cc)

That test is the blocklist and the protected-usage rule (§4). Nothing else.

**(c) Browser-process broker.** `content/browser/hid/hid_service.cc` checks permissions policy, fenced frames,
opaque origins and the per-device permission grant at `Connect()` time
([`hid_service.cc:150-180, 457-467`](https://chromium.googlesource.com/chromium/src/+/main/content/browser/hid/hid_service.cc)).
It contains no visibility, occlusion or focus check, and it has no per-report code path at all — reports flow
directly from the device service to the renderer over a mojo pipe the browser only brokered.

**(d) Renderer dispatch.** The whole of Blink's handling:

```cpp
void HIDDevice::OnInputReport(uint8_t report_id, const Vector<uint8_t>& buffer) {
  DispatchEvent(*MakeGarbageCollected<HIDInputReportEvent>(
      event_type_names::kInputreport, this, report_id, buffer));
}
```
— [`third_party/blink/renderer/modules/hid/hid_device.cc:230-234`](https://chromium.googlesource.com/chromium/src/+/main/third_party/blink/renderer/modules/hid/hid_device.cc)

`HIDDevice` derives from `ExecutionContextLifecycleObserver`, **not** `ExecutionContextLifecycleStateObserver` —
so it never receives freeze/pause state callbacks. Its only lifecycle hook is `ContextDestroyed()`, which clears
pending promise resolvers (`hid_device.cc:421-423`). There is no code in the class that could suppress an event.

**Conclusion:** Chromium implements no focus or visibility gate. This is established by exhaustive reading of the
path, not by a statement in the source.

### 1.3 The scheduler — the only place it could break

An event can still be *delayed* if the task queue carrying it is disabled. This is where the real answer lives.

**The queue.** Both the `HidConnectionClient` receiver and the `HidConnection` remote are bound to
`TaskType::kMiscPlatformAPI` ([`hid_device.cc:279-281, 492-494`](https://chromium.googlesource.com/chromium/src/+/main/third_party/blink/renderer/modules/hid/hid_device.cc)).
`FrameSchedulerImpl::ComputeQueueTraits` maps that task type to `DeferrableTaskQueueTraits()`
([`frame_scheduler_impl.cc:548, 558`](https://chromium.googlesource.com/chromium/src/+/main/third_party/blink/renderer/platform/scheduler/main_thread/frame_scheduler_impl.cc)),
which is:

```cpp
return QueueTraits()
    .SetCanBeDeferred(true)
    .SetCanBeFrozen(true)
    .SetCanBePaused(true)
    .SetCanRunWhenVirtualTimePaused(false)
    .SetCanBePausedForAndroidWebview(true);
```
— [`frame_scheduler_impl.cc:1512-1521`](https://chromium.googlesource.com/chromium/src/+/main/third_party/blink/renderer/platform/scheduler/main_thread/frame_scheduler_impl.cc)

With the defaults from
[`main_thread_task_queue.h:269-284`](https://chromium.googlesource.com/chromium/src/+/main/third_party/blink/renderer/platform/scheduler/main_thread/main_thread_task_queue.h)
(`can_be_throttled = false`, `can_run_in_background = true`), this gives:

| Trait | Value | Consequence for `inputreport` |
| --- | --- | --- |
| `can_be_throttled` | **false** | Background-tab **wake-up throttling does not apply**. Only `CanBeThrottled()` queues join the wake-up and CPU budget pools (`frame_scheduler_impl.cc:268, 316, 343`). The 1-wake-per-minute rule that mangles `setTimeout` in hidden tabs **cannot touch this queue**. |
| `can_run_in_background` | **true** (default) | Not disabled when the frame is not visible (`frame_scheduler_impl.cc:1008`). |
| `can_be_frozen` | **true** | Disabled **only** if the parent page scheduler is frozen (`frame_scheduler_impl.cc:994-1004`). This is the one risk, and it is handled below. |
| `can_be_paused` | true | Only via DevTools / nested-loop pause. Not a production state. |

`UpdateQueuePolicy` disables a queue in exactly three cases — frame paused, page frozen, or
`!frame_visible_ && !CanRunInBackground()` — and only the middle one can apply here.

**So: can the page freeze?** Two independent freezing mechanisms exist, and both refuse.

*Blink's own background-tab freezing is off by default on desktop.* `PageSchedulerImpl::UpdateFrozenState` will
only schedule a freeze when `blink::features::kStopInBackground` is enabled
([`page_scheduler_impl.cc:838-880`](https://chromium.googlesource.com/chromium/src/+/main/third_party/blink/renderer/platform/scheduler/main_thread/page_scheduler_impl.cc)),
and that feature is:

```cpp
BASE_FEATURE(kStopInBackground, "stop-in-background",
#if BUILDFLAG(IS_ANDROID) && !BUILDFLAG(IS_CAST_ANDROID) && !BUILDFLAG(IS_DESKTOP_ANDROID)
             base::FEATURE_ENABLED_BY_DEFAULT
#else
             base::FEATURE_DISABLED_BY_DEFAULT
```
— [`third_party/blink/common/features.cc:2210-2221`](https://chromium.googlesource.com/chromium/src/+/main/third_party/blink/common/features.cc)

Chromium's own comment elsewhere in the same file confirms the intent: *"unlike on desktop, freezing is applied
to most background pages on mobile Android"* (`features.cc:2500-2502`). Even if it were enabled, the grace period
is 1 minute (`page_scheduler_impl.cc:55`) and `IsBackgrounded()` is `!IsPageVisible() && !IsAudioPlaying()`
(`page_scheduler_impl.cc:807-812`) — **a page playing audio is never backgrounded**, and VoxLoop is a voice loop
client that is, by definition, playing audio.

*Chrome's browser-side freezing policy explicitly exempts HID.* `FreezingPolicy` is installed unconditionally on
all non-Android platforms
([`chrome_browser_main_extra_parts_performance_manager.cc:240-242`](https://chromium.googlesource.com/chromium/src/+/main/chrome/browser/performance_manager/chrome_browser_main_extra_parts_performance_manager.cc)).
Its enumeration of reasons a page cannot be frozen includes, verbatim:

```cpp
case CannotFreezeReason::kConnectedToHidDevice:
  return "connected to HID device";
```
— [`components/performance_manager/freezing/cannot_freeze_reason.cc:33-34`](https://chromium.googlesource.com/chromium/src/+/main/components/performance_manager/freezing/cannot_freeze_reason.cc)

set by `FreezingPolicy::OnIsConnectedToHidDeviceChanged`
([`freezing_policy.cc:70, 929-933`](https://chromium.googlesource.com/chromium/src/+/main/components/performance_manager/freezing/freezing_policy.cc)),
fed from `PageLiveStateDecorator::IsConnectedToHidDevice`
([`page_live_state_decorator.cc:565-569`](https://chromium.googlesource.com/chromium/src/+/main/components/performance_manager/decorators/page_live_state_decorator.cc)),
which is driven by `WebContentsImpl::IncrementHidActiveFrameCount()` — called the moment the first WebHID
connection opens (`content/browser/hid/hid_service.cc:465-467, 517-522`).

A VoxLoop console would additionally trip `kAudible`, `kWebRTC` and `kCapturingAudio` from the same list. Four
independent reasons; any one is sufficient.

*And the back/forward cache is disabled too.* Creating a `HidService` registers a bfcache-disabling non-sticky
feature:

```cpp
back_forward_cache_feature_handle_ =
    render_frame_host->RegisterBackForwardCacheDisablingNonStickyFeature(
        blink::scheduler::WebSchedulerTrackedFeature::kWebHID);
```
— [`content/browser/hid/hid_service.cc:86-97`](https://chromium.googlesource.com/chromium/src/+/main/content/browser/hid/hid_service.cc),
gated on `kWebHidAttributeAllowsBackForwardCache`, which is `FEATURE_ENABLED_BY_DEFAULT`
([`services/device/public/cpp/device_features.cc:71-74`](https://chromium.googlesource.com/chromium/src/+/main/services/device/public/cpp/device_features.cc)).
A page using WebHID cannot enter the bfcache, which removes the other route to a frozen frame.

### 1.4 The four states, distinguished

The ticket asked specifically about the difference between a backgrounded tab and a focused-elsewhere window.
Chromium's answer is that only one of the four states below is even a *visibility* change, and none of them is a
*focus* check.

| Operator state | Chromium page state | `inputreport` |
| --- | --- | --- |
| VoxLoop window visible, telemetry app focused | `Visibility::VISIBLE` → `PageVisibilityState::kVisible` | **Delivered, no change of any kind.** Keyboard focus is not an input to page visibility. This is VoxLoop's real scenario. |
| VoxLoop tab in a background tab-strip position | `kHidden` | Delivered. Queue is non-throttleable; page cannot freeze (§1.3). |
| VoxLoop window fully covered by another window | `Visibility::OCCLUDED` → `kHidden` (`web_contents_impl.cc:5107-5130, 12426`) | Delivered. Same reasoning. |
| VoxLoop window minimised | `kHidden` | Delivered. Same reasoning. |

**Does the connection itself survive?** Yes. `HIDDevice` only tears down on `ContextDestroyed` — document
destruction or navigation. The `HidService` is a `DocumentService` destroyed when the document is destroyed or
navigates (`hid_service.cc:179-183`). Visibility changes are not in that set. The device handle lives in the
browser / device-service process and is unaffected by renderer state entirely. `HIDDevice::HasPendingActivity()`
returns true while connected with a listener attached, which keeps the object alive against GC
(`hid_device.cc:425-429`).

### 1.5 Where the trail ends — read this before treating §1 as settled

I need to be precise about the epistemic status of the above, because the ticket asked for it.

- **No primary source states the unfocused case in prose.** Not the spec, not Chrome's own developer
  documentation (which I checked directly — [developer.chrome.com/docs/capabilities/hid](https://developer.chrome.com/docs/capabilities/hid)
  covers permissions, protected usages and Linux udev rules, and says **nothing** about focus or visibility), and
  not any Chromium design doc I could reach.
- **No test asserts it.** The entire web-platform-tests `webhid/` directory is five files, all about permissions
  policy and IDL harness ([wpt `webhid/`](https://github.com/web-platform-tests/wpt/tree/master/webhid)).
  Chromium's own `content/browser/hid/hid_browsertest.cc` has six tests — `GetDevices`, `RequestDevice`,
  `DisallowRequestDevice`, `ProtectedReportsAreFiltered`, `DeviceWithAllProtectedReportsIsExcluded`,
  `BlockFromFencedFrame`. None touches visibility, occlusion, backgrounding or freezing.
- **The conclusion is therefore "nothing suppresses it", not "something guarantees it".** Absence of a gate is
  strong evidence — I read every layer — but it is not a contract. A future Chromium change could add one, and
  the spec would not forbid it. The `kConnectedToHidDevice` freezing exemption is the closest thing to a
  deliberate commitment, and it is a Chrome-side policy, not a web-platform guarantee.
- **Latency is unmeasured.** A hidden renderer is normally lowered to `base::Process::Priority::kBestEffort`,
  but `RenderProcessPriority::is_background()` returns false when `has_media_stream` is set
  ([`content/browser/child_process_launcher.cc:422-424`](https://chromium.googlesource.com/chromium/src/+/main/content/browser/child_process_launcher.cc),
  [`child_process_launcher.h:126-134`](https://chromium.googlesource.com/chromium/src/+/main/content/browser/child_process_launcher.h)) —
  so a page holding a live `getUserMedia` capture keeps `kUserBlocking` priority even when hidden. That should
  make PTT latency in a hidden VoxLoop tab indistinguishable from a visible one, but *should* is doing work in
  that sentence and #5 established that a PTT path dropping the first 200 ms is an operational failure. **Measure
  it in the spike.**
- **Nothing here says anything about a locked screen, a switched user session, or a screensaver.** Not
  investigated; not established.

**openvocs is not evidence for the unfocused case.** Its PTT handler (`ptt_bar.js:214-252`) opens every
previously-granted device and treats `event.data.getUint8(0) === 32` as a press — it never inspects the usage
page, and the chooser filter is `HID_FILTERS = []`, i.e. empty
([`views/auth/js/default_config.js:55`](https://github.com/openvocs/openvocs/blob/main/src/HTML/app/vocs/views/auth/js/default_config.js),
[`views/loops/js/ui/settings_slider.js:154`](https://github.com/openvocs/openvocs/blob/main/src/HTML/app/vocs/views/loops/js/ui/settings_slider.js)).
It proves HID PTT works in a browser. It proves nothing about focus, and its byte-comparison approach is a
fragile pattern VoxLoop should not copy.

---

## 2. The permission model

### 2.1 What the grant costs to obtain

Per spec, `requestDevice()` requires **transient activation** (a user gesture) and rejects with `SecurityError`
without it; it also requires a `Window` global (rejecting with `NotSupportedError` in a worker) and the `"hid"`
permissions-policy feature, whose default allowlist is `'self'`
([spec §6.2](https://wicg.github.io/webhid/#requestdevice-method),
[§16.1](https://wicg.github.io/webhid/#permissions-policy)). Chromium adds `[SecureContext]` on the IDL
(`third_party/blink/renderer/modules/hid/navigator_hid.idl`), so HTTPS or localhost only — relevant for a
self-hosted deployment, which will need a real certificate on the VoxLoop origin.

`getDevices()` needs **no gesture** — only the permissions-policy check — and returns every device the origin has
been granted, per spec §6.1 and Chrome's own docs (*"a list of devices the website has been granted access to
previously"*). That is the shift-start path, and it is the right one.

### 2.2 Whether the grant survives — the finding that bites

Chromium stores WebHID grants through `HidChooserContext`, an `ObjectPermissionContextBase` over the
`HID_GUARD` / `HID_CHOOSER_DATA` content settings
([`chrome/browser/hid/hid_chooser_context.cc:117-124`](https://chromium.googlesource.com/chromium/src/+/main/chrome/browser/hid/hid_chooser_context.cc)).
Whether a grant persists turns on one predicate:

```cpp
bool HidChooserContext::CanStorePersistentEntry(
    const device::mojom::HidDeviceInfo& device) {
  return !device.serial_number.empty() && !device.product_name.empty();
}
```
— `hid_chooser_context.cc:175-178`

and the comment beside it is unambiguous:

> Use the USB serial number as a persistent identifier. If it is unavailable, **only ephemeral permissions may be
> granted.** […] The GUID is a temporary ID created on connection that remains valid until the device is
> disconnected. **Ephemeral permissions are keyed by this ID and must be granted again each time the device is
> connected.**
> — `hid_chooser_context.cc:148-158`

| Device exposes a serial-number string? | Grant | Survives page reload | Survives browser restart | Survives unplug/replug | Survives reboot |
| --- | --- | --- | --- | --- | --- |
| **Yes** (and a non-empty product name) | Persistent, written to the profile's content settings, matched on `vendorId` + `productId` + `serialNumber` (`hid_chooser_context.cc:518-534`) | Yes | Yes | Yes | Yes |
| **No** | Ephemeral, held in an in-memory `ephemeral_devices_` map keyed by connection GUID (`hid_chooser_context.cc:393-411`) | Yes | **No** | **No** — `DeviceRemoved()` revokes it on disconnect (`hid_chooser_context.cc:656-670`) | **No** |

**This is the control-room risk.** Cheap footswitches and many headsets ship without a serial-number string. On
such a device an operator would have to click through the chooser at the start of every shift, and again after
any unplug. That is exactly the usability failure #16 flagged, and it is a **property of the device, not of the
browser** — so it becomes a procurement criterion.

### 2.3 The fix: enterprise policy pre-grants

Chromium ships four Chrome-100+ policies that grant WebHID access **with no gesture, no chooser and no
persistence question at all**, on all desktop platforms:

| Policy | Grants by | Since |
| --- | --- | --- |
| [`WebHidAllowDevicesForUrls`](https://chromium.googlesource.com/chromium/src/+/main/components/policy/resources/templates/policy_definitions/ContentSettings/WebHidAllowDevicesForUrls.yaml) | `vendor_id` (+ optional `product_id`) per URL | `chrome.*:100-`, `chrome_os:100-` |
| [`WebHidAllowDevicesWithHidUsagesForUrls`](https://chromium.googlesource.com/chromium/src/+/main/components/policy/resources/templates/policy_definitions/ContentSettings/WebHidAllowDevicesWithHidUsagesForUrls.yaml) | `usage_page` (+ optional `usage`) per URL | `chrome.*:100-` |
| `WebHidAllowAllDevicesForUrls` | any device, per URL | `chrome.*:100-` |
| `DefaultWebHidGuardSetting`, `WebHidAskForUrls`, `WebHidBlockedForUrls` | the guard content setting | — |

The caption is literal: *"Automatically grant permission to these sites to connect to HID devices with the given
vendor and product IDs."* In `HidChooserContext::HasDevicePermission` the policy check short-circuits ahead of
both the ephemeral map and the stored objects (`hid_chooser_context.cc:497-501`), so a policy-granted device is
simply *always* permitted — `getDevices()` returns it on first load, no gesture ever, across every restart.

The blocklist check runs **before** the policy check (`hid_chooser_context.cc:480-495`), so policy cannot unblock
a blocklisted device.

**Operationally**: on a managed Windows estate this is a GPO/registry value; on managed Linux it is a JSON file
in `/etc/opt/chrome/policies/managed/`. For an unmanaged pilot machine, a persistent grant on a serial-numbered
device is the fallback, and a non-serial device means a per-shift click. Chrome also shows a persistent "in use"
indicator whenever a connection is open (spec §3.1 mitigation; `IncrementHidActiveFrameCount`) — that is a
permanent piece of chrome in the operator's view and cannot be suppressed.

---

## 3. Browser reach

### 3.1 Engines

| Engine | Status | Primary source |
| --- | --- | --- |
| **Chromium** | Shipped, enabled by default, **Chrome 89**. Dedicated-worker support in 131; extension-service-worker support in 117; `exclusionFilters` in 102; `forget()` in 100. | [chromestatus feature 5172464636133376](https://chromestatus.com/feature/5172464636133376) |
| **Gecko / Firefox** | **Not implemented, position negative.** `position: negative`, rationale: *"This API, like WebUSB, provides access to generic devices. […] devices are generally not designed with access from arbitrary websites in their threat model."* The position was landed as a PR literally titled **"Record WebHID as harmful"** (merged 2021-08-26). | [mozilla/standards-positions `activities.yml`](https://github.com/mozilla/standards-positions/blob/main/activities.yml), [issue 459](https://github.com/mozilla/standards-positions/issues/459), [PR 558](https://github.com/mozilla/standards-positions/pull/558) |
| **WebKit / Safari** | **No position, no implementation.** Request filed 2025-06-10; `"position": null` in the summary, flagged `concerns: venue`. There is no `Source/WebCore/Modules/hid` in the WebKit tree. | [WebKit/standards-positions issue 510](https://github.com/WebKit/standards-positions/issues/510), [`summary.json`](https://github.com/WebKit/standards-positions/blob/main/summary.json) |

Mozilla Bugzilla has **no implementation bug** for WebHID — only wpt-sync bookkeeping bugs and a growing set of
open web-compat reports: [1835412](https://bugzilla.mozilla.org/show_bug.cgi?id=1835412) *"Sites breaking due to
the lack of WebHID support"* (NEW, Core :: DOM: Device Interfaces), plus site reports 1974547, 2031980, 2044346,
2054418. The spec's non-Recommendation status (§1.1) and Mozilla's negative position are the same fact seen from
two sides: this is a single-vendor API and will stay one.

**For VoxLoop this means: Chrome or Edge, mandated.** That is a deployment constraint to write into the spec, not
a detail. Edge, Brave, Opera and Vivaldi inherit it from Chromium; Firefox and Safari users get nothing.

### 3.2 Linux: the udev requirement is real and undocumented in the spec

On non-ChromeOS Linux, Chromium opens the hidraw node with its own credentials, with no privileged helper:

```cpp
base::FilePath device_path(params->device_info->device_node());
int flags = base::File::FLAG_OPEN | base::File::FLAG_READ | base::File::FLAG_WRITE;
device_file.Initialize(device_path, flags);
if (!device_file.IsValid()) {
  if (file_error == base::File::FILE_ERROR_ACCESS_DENIED) {
    HID_LOG(EVENT) << "Access denied opening device read-write, trying read-only.";
    flags = base::File::FLAG_OPEN | base::File::FLAG_READ;
    device_file.Initialize(device_path, flags);
  }
}
```
— [`services/device/hid/hid_service_linux.cc:452-486`](https://chromium.googlesource.com/chromium/src/+/main/services/device/hid/hid_service_linux.cc)

(ChromeOS is the exception: it routes through `PermissionBrokerClient::OpenPath`, `hid_service_linux.cc:407-418`.)

**Default permissions are `0600 root:root`.** The kernel's `hidraw_class` declares no `.devnode` callback that
would set a permissive mode ([`drivers/hid/hidraw.c`](https://github.com/torvalds/linux/blob/master/drivers/hid/hidraw.c)),
and systemd's shipped rules tag `SUBSYSTEM=="hidraw"` with `uaccess` for only four narrow categories —
`ID_AV_PRODUCTION_CONTROLLER`, `ID_AV_LIGHTS`, `ID_HARDWARE_WALLET`, `ID_INPUT_3D_MOUSE`
([`70-uaccess.rules.in:112-130`](https://github.com/systemd/systemd/blob/main/rules.d/70-uaccess.rules.in)) —
plus FIDO tokens via `60-fido-id.rules`. **Headsets, footswitches and PTT boxes are in none of those
categories.**

Chrome's own documentation states the requirement plainly, under *Dev tips*:

> On most Linux systems, HID devices are mapped with read-only permissions by default. To allow Chrome to open an
> HID device, you will need to add a new udev rule.
> — [developer.chrome.com/docs/capabilities/hid](https://developer.chrome.com/docs/capabilities/hid)

with the shape:

```
# /etc/udev/rules.d/70-voxloop-ptt.rules
KERNEL=="hidraw*", ATTRS{idVendor}=="<vid>", ATTRS{idProduct}=="<pid>", MODE="0660", GROUP="plugdev"
```

(`TAG+="uaccess"` is the more modern alternative and avoids group membership management on systemd systems.)

**So a Linux VoxLoop deployment ships three things:** a udev rule per approved PTT device, an enterprise policy
JSON in `/etc/opt/chrome/policies/managed/`, and a documented Chrome/Edge requirement. None of that is hard; all
of it has to be *decided* and packaged. On Windows nothing is required beyond the policy — `CreateFile` on the
HID collection needs no elevation.

---

## 4. Device reality

### 4.1 What Chromium will and will not let a page read

Two filters apply, and both run in the browser/device-service process where a page cannot influence them.

**Filter 1 — always-protected usages.** This is the load-bearing one:

```cpp
bool IsAlwaysProtected(const mojom::HidUsageAndPage& hid_usage_and_page,
                       HidReportType report_type) {
  if (usage_page == mojom::kPageKeyboard) return true;                     // 0x07, entire page
  if (usage_page != mojom::kPageGenericDesktop) return false;              // 0x01
  if (usage == kGenericDesktopPointer || usage == kGenericDesktopMouse ||
      usage == kGenericDesktopKeyboard || usage == kGenericDesktopKeypad)  // 0x01,0x02,0x06,0x07
    return report_type != HidReportType::kFeature;
  if (usage >= kGenericDesktopSystemControl && usage <= kGenericDesktopSystemWarmRestart) return true;  // 0x80–0x8F
  if (usage >= kGenericDesktopSystemDock && usage <= kGenericDesktopSystemDisplaySwap) return true;     // 0xA0–0xB6
  return false;
}
```
— [`services/device/public/cpp/hid/hid_report_utils.cc:36-66`](https://chromium.googlesource.com/chromium/src/+/main/services/device/public/cpp/hid/hid_report_utils.cc),
constants from [`services/device/public/mojom/hid.mojom:14-120`](https://chromium.googlesource.com/chromium/src/+/main/services/device/public/mojom/hid.mojom)

The WebHID path always passes `allow_protected_reports=false` (`content/browser/hid/hid_service.cc:479`), so this
is never relaxed for a web page.

**Filter 2 — the WICG blocklist**, fetched from
[`WICG/webhid/blocklist.txt`](https://github.com/WICG/webhid/blob/main/blocklist.txt). It is 49 lines. In full,
the active rules block: FIDO (`usagePage:0xF1D0`); Generic Desktop Mouse / Keyboard / Keypad; Generic Desktop
System Control; one Jabra proprietary **output** report (`vendor:0x0b0e, usagePage:0xff00, reportId:0x05,
reportType:"output"`); and all OnlyKey devices (`vendor:0x1d50, product:0x60fc`).

Its comment on the keyboard rules is the single most useful sentence in any of these sources for VoxLoop:

> HID access to mouse and keyboard inputs enables the creation of input loggers and **allows web pages to subvert
> the operating system's focus model.** Block collections that expose this type of functionality.

That is the spec repository stating, as the reason for the restriction, that WebHID **does** bypass focus. It is
not a delivery guarantee, but it is the clearest confirmation available that focus-independence is a known and
intended property of the API for the usages that remain allowed.

### 4.2 The device table

| Device class | Typical top-level collection | Readable via WebHID? |
| --- | --- | --- |
| **Headset inline PTT / hook-switch / mute button** (Jabra, Poly/Plantronics, Sennheiser/EPOS) | Telephony Device page `0x0B` (Hook Switch `0x20`, Phone Mute `0x2F`), sometimes Consumer `0x0C`, sometimes vendor `0xFF00` | **Yes.** None of these pages is in `IsAlwaysProtected`. This is what openvocs uses. Note the one Jabra blocklist rule affects an *output* report only — button presses are unaffected. |
| **Dedicated PTT box / radio-style desk switch** | Almost always vendor-defined `0xFF00+`, sometimes Button page `0x09` | **Yes.** Vendor pages are unrestricted. |
| **Footswitch that enumerates as a joystick or gamepad** | Generic Desktop Joystick `0x04` / GamePad `0x05` | **Yes.** Not in the protected list. |
| **Footswitch that emits plain keystrokes** — i.e. the "works for free" device from #5 | Keyboard `0x07`, or Generic Desktop Keyboard `0x06` | **No. Blocked, and invisible.** Every input report is protected, and a device whose reports are *all* protected is not even returned by `getDevices()` — asserted by Chromium's own test `DeviceWithAllProtectedReportsIsExcluded` ([`hid_browsertest.cc:172-188`](https://chromium.googlesource.com/chromium/src/+/main/content/browser/hid/hid_browsertest.cc)). |
| **Composite device** (keyboard collection *plus* a vendor collection) | both | **Partially.** The device appears; keyboard reports are stripped, the vendor collection is readable — asserted by `ProtectedReportsAreFiltered` (`hid_browsertest.cc:144-170`). Many programmable footswitches are of this shape and are the sweet spot. |
| **Any device also opened exclusively by the OS** (system keyboards/mice on Windows) | — | **No**, at the OS layer, before Chromium's rules apply. |

### 4.3 What HID actually adds over #5's keystroke footswitch

This is the question the ticket asked, and the answer is sharper than expected:

- **It adds the entire class of devices that are not keyboards** — headset inline buttons, telephony hook
  switches, and vendor-defined PTT boxes. #5's conclusion that *"headset PTT buttons are uncatchable as
  keystrokes"* was correct and remains correct; they are catchable as HID reports. That correction stands.
- **It adds unambiguous press *and* release** in the report payload, with no dependence on OS key-repeat, no
  `WM_HOTKEY` press-only limitation (#5, §2.2) and no 50 ms `GetAsyncKeyState` release poll. For momentary PTT
  this is materially better than the Windows global-hotkey path.
- **It adds no conflict with the telemetry application.** A vendor-page report is meaningless to every other
  application on the machine, so the key-conflict problem in #9 disappears for HID devices — nothing else is
  listening on that report.
- **It removes the keystroke footswitch.** The device that "worked for free" under a global hotkey is precisely
  the device WebHID refuses to expose. A browser-tier PTT built on WebHID **cannot** use a keyboard-emulating
  footswitch, at all, under any configuration short of `--disable-hid-blocklist`
  ([`hid_switches.cc`](https://chromium.googlesource.com/chromium/src/+/main/services/device/public/cpp/hid/hid_switches.cc)) —
  and that switch does not relax `IsAlwaysProtected`, which is separate from the blocklist and unconditional.

So the two PTT mechanisms are complements at the *architecture* level and substitutes at the *device* level. A
site that has already bought keystroke footswitches gets nothing from WebHID; a site buying headsets with inline
PTT gets everything. **Procurement guidance falls straight out: buy devices that expose a Telephony, Consumer,
vendor-defined or joystick collection, and that report a USB serial number string** (§2.2).

---

## 5. WebHID inside Tauri's WebView2 on Windows

Added to scope after the note was drafted, because Ed has proposed making the browser the first-class v1 client
with a thin Tauri wrapper added later for Windows deployment. The question: would that wrapper *lose* the
peripheral capability the plain browser has?

**Verdict: not established, and the evidence leans towards "partly, yes".** `navigator.hid` is present in
WebView2. Nothing I can read demonstrates that `requestDevice()` actually works there, the failure mode if it
does not is **silent**, and — separately and more certainly — **the two mechanisms that make WebHID deployable in
a control room are Chrome-browser-layer features that WebView2 does not expose and Tauri cannot reach.**

### 5.1 The embedding surface: why this is a real question, not paranoia

WebHID is not part of Chromium's content layer by default. `content/browser/hid/hid_service.cc` refuses to create
the service if the embedder has not supplied a delegate:

```cpp
// Avoid creating the HidService if there is no HID delegate to provide the
// implementation.
if (!GetContentClient()->browser()->GetHidDelegate())
  return;
```
— [`content/browser/hid/hid_service.cc:157-160`](https://chromium.googlesource.com/chromium/src/+/main/content/browser/hid/hid_service.cc)

and the base implementation returns nothing:

```cpp
HidDelegate* ContentBrowserClient::GetHidDelegate() {
  return nullptr;
}
```
— [`content/public/browser/content_browser_client.cc:1243-1245`](https://chromium.googlesource.com/chromium/src/+/main/content/public/browser/content_browser_client.cc)

Every embedder must implement it deliberately. Electron does, in open source
([`shell/browser/electron_browser_client.cc:2035-2039`](https://github.com/electron/electron/blob/main/shell/browser/electron_browser_client.cc)),
and documents the host-app work required to make it function — a `select-hid-device` handler and optionally
`setDevicePermissionHandler` ([Electron `docs/tutorial/devices.md:36-57`](https://github.com/electron/electron/blob/main/docs/tutorial/devices.md)).
**WebView2 is closed source and I cannot read whether it does.**

### 5.2 The failure mode is silent — which is why "navigator.hid exists" proves nothing

If the delegate is missing, `HidService::Create` returns without binding the mojo receiver. The pipe closes,
Blink's disconnect handler runs, and:

```cpp
void HID::CloseServiceConnection() {
  service_.reset();
  receiver_.reset();
  // ...
  for (HIDDeviceResolver* resolver : get_devices_promises)     resolver->Resolve(HeapVector<Member<HIDDevice>>());
  for (HIDDeviceResolver* resolver : request_device_promises)  resolver->Resolve(HeapVector<Member<HIDDevice>>());
}
```
— [`third_party/blink/renderer/modules/hid/hid.cc:497-514`](https://chromium.googlesource.com/chromium/src/+/main/third_party/blink/renderer/modules/hid/hid.cc)

Both `getDevices()` and `requestDevice()` **resolve with an empty array**. No rejection, no `DOMException`, no
console warning. That is byte-for-byte identical to "the user dismissed the chooser" and to "no devices are
paired yet".

And `navigator.hid` itself is exposed independently of all this — it is gated only on the Blink runtime flag
`WebHID`, whose status is `{"Android": "", "default": "stable"}`
([`runtime_enabled_features.json5:7005-7007`](https://chromium.googlesource.com/chromium/src/+/main/third_party/blink/renderer/platform/runtime_enabled_features.json5)),
i.e. on for every non-Android Chromium build regardless of embedder. **So `!!navigator.hid` — the exact feature
detection openvocs uses (`ptt_bar.js:215`) — cannot distinguish a working WebHID from a completely inert one.**
A Tauri build could ship, report "HID supported", show an empty device list, and look to an operator like a
pairing problem rather than a missing platform feature.

### 5.3 What the primary sources actually say

| Source | What it establishes |
| --- | --- |
| [tauri-apps/tauri#14549](https://github.com/tauri-apps/tauri/issues/14549) — *"[feat] webhid API"*, opened 2023-11-16 | A user reports on 2025-08-13, with a devtools screenshot, that *"The **hid** object is exposed by the navigator object in the Tauri WebView."* Tauri core maintainer **FabianLars** replies: *"yes, and probably always was since the windows webview is based on chromium. this issue really is about the other platforms."* Asked directly whether WebView2 has *"any unexpected restrictions regarding the WebHID API"*, he answers: ***"I'm not aware of any restrictions compared to any other chromium based browser."*** That is an absence-of-knowledge statement, not a test result. **Nobody in the thread reports a successful `requestDevice()` call.** The issue was transferred out of the repo and locked on 2025-11-23 without resolution. |
| [tauri-apps/tauri#13242](https://github.com/tauri-apps/tauri/issues/13242) — *"Support for persistent WebHID device authorization"*, opened and closed 2025-04-16 | The same maintainer, on the host-side API: ***"Seems like WebView2 doesn't expose anything even closely related to webhid so this option is out of the question for now (there isn't even a permission event like in webrtc apparently)."*** Closed as a duplicate of #14549. **Tauri has no WebHID support of its own** — no plugin, no permission event, no persistence hook. |
| [`CoreWebView2PermissionKind`](https://learn.microsoft.com/en-us/dotnet/api/microsoft.web.webview2.core.corewebview2permissionkind), SDK 1.0.4129.50 (page updated 2026-08-04) | Thirteen members: `UnknownPermission`, `Microphone`, `Camera`, `Geolocation`, `Notifications`, `OtherSensors`, `ClipboardRead`, `MultipleAutomaticDownloads`, `FileReadWrite`, `Autoplay`, `LocalFonts`, `MidiSystemExclusiveMessages`, `WindowManagement`, `PersistentStorage`. **None relates to HID, USB, Serial or Bluetooth.** Corroborates the maintainer's observation exactly. |
| [*Differences between Microsoft Edge and WebView2*](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/browser-features) (updated 2026-06-12) | Its *"Web platform features currently unavailable"* table lists exactly three entries: **Push Notifications, Web Payment API, Periodic Background Sync.** WebHID is not listed as unavailable — and is not listed as available either. The page never mentions HID, USB, Serial, Bluetooth or device choosers. Absence from an incomplete list is not evidence in either direction. |
| WebView2 API reference, searched for any device-chooser surface | No `HidDelegate` analogue, no device-chooser event, no `SetDevicePermissionHandler` equivalent. |

### 5.4 The part that *is* established, and it is worse than the framing assumed

Even if `requestDevice()` works in WebView2, **the two things that make WebHID deployable in a control room live
in `//chrome`, not `//content`, and a Tauri wrapper gets neither:**

- **Persistent permissions.** `HidChooserContext` — the whole of §2.2's persistence story — is
  `chrome/browser/hid/hid_chooser_context.cc`, an `ObjectPermissionContextBase` over Chrome's
  `HID_GUARD` / `HID_CHOOSER_DATA` content settings. It is Chrome-browser code. An embedder that wants grants to
  survive a restart must reimplement it. Electron's documentation states its own default plainly: *"the default
  behavior of Electron is to store granted device permission **through the lifetime of the corresponding
  WebContents**"* — i.e. no persistence at all unless the host app writes it. Tauri exposes no hook to write it,
  which is precisely what issue #13242 asked for and did not get.
- **Enterprise pre-grants.** `WebHidAllowDevicesForUrls` and friends are read through
  `HidPolicyAllowedDevicesFactory` in `chrome/browser/hid/` from `prefs::kManagedWebHidAllowDevicesForUrls`
  ([`hid_policy_allowed_devices.cc:35-71`](https://chromium.googlesource.com/chromium/src/+/main/chrome/browser/hid/hid_policy_allowed_devices.cc)).
  Chrome browser policy. **A WebView2-hosted app is not Chrome and does not read Chrome's policies**, so §2.3's
  clean shift-start answer — no gesture, no chooser, ever — is a browser-only capability.

So the counterintuitive outcome the coordinator anticipated is not only real but wider than framed: the Tauri
wrapper's risk is not just "WebHID might not work", it is **"even where it works, the deployment ergonomics
regress from Chrome's"**.

Two things narrow the blast radius. First, this bites *only* for HID-only peripherals — headset inline buttons
and vendor-page PTT boxes. Plain keyboard PTT is covered natively on Windows by `global-hotkey` / `RegisterHotKey`
(#5 §2.2), which needs no webview involvement at all. Second, it is Windows-and-macOS-shaped: on Linux the
question does not arise, because #5 established that a Tauri webview there has no `RTCPeerConnection` and so
cannot be a VoxLoop client at all.

### 5.5 What would settle it

Add ten minutes to the §0 spike: build a minimal Tauri app on Windows, load a page that calls
`navigator.hid.requestDevice({filters: []})` from a button click, and observe whether a chooser appears. Three
outcomes, all decisive:

1. **A chooser appears and a device connects** — WebView2 implements a HID delegate; then re-test persistence by
   restarting the app, because that is a separate question with a separate answer.
2. **The promise resolves with `[]` and no chooser appears** — no delegate. WebHID is inert in Tauri on Windows,
   and a Tauri wrapper strictly loses a capability the browser has.
3. **A chooser appears but the grant is gone after restart** — the likeliest middle case, and the one that
   matters operationally: functional in a demo, unusable on a shift.

Until that is run, **treat WebHID as a browser-tier capability only**, and do not assume the Tauri wrapper
inherits it.

---

## 6. Recommendation

**Adopt WebHID as the browser tier's primary hardware PTT path, and stop treating "PTT needs a desktop app" as
settled — but run the spike before rewriting the map.**

1. **Run the spike first.** Ninety minutes with a real headset or PTT box on a real Windows machine, testing the
   four states in §1.4 plus a timestamped latency measurement while hidden. The source says it works; nothing in
   the source is a promise. Re-type #16's follow-up as a prototype ticket, as #16 itself anticipated.
2. **If the spike confirms it, the browser tier gets three PTT inputs, in preference order**: (a) a WebHID device
   — unfocused, momentary and latched, no key conflicts; (b) an on-screen button — always available, focused
   only; (c) a keyboard shortcut — focused only, and it is *only* focused-only, which is the degradation story
   #9 asks for.
3. **Make the device the deployment variable, not the client.** Specify a supported-device list with two hard
   criteria — a non-keyboard top-level collection, and a USB serial-number string — plus the udev rule and the
   `WebHidAllowDevicesForUrls` policy fragment shipped alongside it. A device that fails either criterion falls
   back to on-screen PTT, degraded but honest.
4. **Do not build the HID handler the way openvocs did.** `getUint8(0) === 32` against an unfiltered device list
   is a per-device magic number. VoxLoop should filter `requestDevice()` by usage page, parse the report
   descriptor exposed on `HIDDevice.collections`, and bind to a named usage — which is the difference between
   "works on the device we tested" and "works on the device the customer bought".
5. **Do not assume a Tauri wrapper inherits any of this.** Until the §5.5 test is run, WebHID is a *browser-tier*
   capability. Add the ten-minute WebView2 check to the same spike.
6. **The desktop tier's justification is now narrower, not gone.** See §7.

---

## 7. What this changes for VoxLoop

### For [#9 — client tiers and push-to-talk](https://github.com/edwardhutchinson/voxloop/issues/9)

- **The capability split as written is wrong.** #9 proposes "operators run Tauri (unfocused PTT); browser gets
  on-screen PTT and latch only". If the spike confirms §1, the browser tier gets **unfocused, momentary,
  hardware PTT** — the single capability the split was built around. The honest split becomes *hardware-dependent*
  rather than *tier-dependent*: with a supported HID device, browser and desktop are equivalent for PTT; without
  one, the browser is focused-only and the desktop app is not.
- **"Is Tauri confirmed?" gets a real answer: not on this evidence alone.** The map's stated reason for the
  desktop tier — *"a web page cannot see global hotkeys"* — is true but no longer sufficient, because VoxLoop
  does not need a global hotkey if it has a HID device. Tauri's remaining independent justifications are:
  keystroke-emulating footswitches (which WebHID cannot read), arbitrary keyboard PTT while unfocused, and
  anything else in the map that wants a native process. Those should be argued on their own merits.
- **Peripherals move from "defer" to "decide now."** §4 turns peripheral support into a procurement question with
  two testable criteria, and the answer changes the client architecture. It cannot be deferred past #9.
- **The key-conflict question in #9 partly dissolves.** A vendor-page HID report cannot collide with the
  telemetry application's shortcuts. That only holds for the HID path; keyboard PTT keeps the conflict.
- **Momentary vs latched is unaffected.** HID gives clean press and release, so both modes bind naturally from
  the same report stream — better than the Windows `WM_HOTKEY` path #5 documented.
- **"Browser first, thin Tauri wrapper later" is the right shape, but the wrapper is not free.** §5 establishes
  that a WebView2 wrapper does not inherit Chrome's WebHID persistence or its enterprise pre-grant policies, and
  may not have a functioning `navigator.hid` at all. The capability matrix for #9 therefore needs a **third**
  column — browser / Tauri-Windows / Tauri-Linux — not two, and the Tauri-Windows column has an open cell that
  only a build can fill. Notably it is HID-only peripherals that are at risk; keyboard PTT is covered natively by
  `global-hotkey` on Windows, so the wrapper is *strictly better* for keyboards and *possibly worse* for
  headsets. That inversion is worth stating explicitly in the ADR.

### For [#12 — operator console UI](https://github.com/edwardhutchinson/voxloop/issues/12)

- **A first-run and per-shift device-pairing flow now exists and must be designed.** At minimum: a settings pane
  listing granted devices from `getDevices()`, a "pair a device" button that calls `requestDevice()` from a real
  click, and a visible state for "no PTT device paired".
- **The ephemeral-permission case needs an explicit, non-confusing UI state.** If the site's device has no serial
  number, the operator is re-pairing every shift and must be told why, in those words, rather than discovering a
  dead PTT button mid-event. Given the map's "state shown must always be factual" rule, a paired-but-unopened
  device must never render as ready.
- **Chrome's own "HID device in use" indicator is permanent chrome in the operator's view** and cannot be
  suppressed. Worth knowing before designing around browser chrome.
- **A hidden VoxLoop window still transmits.** If PTT works while the console is behind the telemetry app, the
  operator may be talking with no VoxLoop pixels visible at all. That is a genuine new cognitive-load hazard —
  the latched-open-mic problem #9 already calls the highest-risk failure, made worse by the window being
  invisible. Some out-of-window signal (a system notification, an audio cue, or a deliberate refusal to allow
  *latched* mode while hidden) needs designing. **This is the one place where a "yes" makes the product harder,
  not easier.**

### Does it weaken the case for the Tauri desktop tier?

**Yes, substantially — but it does not remove it, and it does nothing at all for Linux.**

It weakens it because the load-bearing premise ("PTT must work unfocused, and only a native app can do that") is
false for HID devices. If the pilot site's operators wear headsets with inline PTT, a browser tab does the job,
and the Tauri tier's cost — packaging, code signing, auto-update on a possibly air-gapped network, a second shell
to maintain — buys correspondingly less.

It does not remove it, because: WebHID is Chromium-only and Mozilla has recorded it as harmful; keystroke
footswitches remain unreadable; and this note establishes an absence of gating, not a guaranteed behaviour.

**And it cuts the other way too.** If Ed's "browser first, Tauri wrapper for Windows deployment later" plan is
adopted, §5 says the wrapper must be justified on packaging, installation and integration grounds *alone* — it
may well subtract from the browser's peripheral story rather than add to it. A Tauri tier whose only remaining
job is "put the browser in a window with an installer" is a legitimate thing to want; it is just no longer the
thing that makes PTT work.

**And it is explicitly out of scope for the Linux problem.** #5's second wall — WebKitGTK as shipped by Debian,
Fedora and Arch is compiled without WebRTC, so a Tauri webview on Linux has no `RTCPeerConnection` — is a property
of the Tauri webview and is untouched by anything in this note. A finding about Chrome's WebHID implementation
says nothing about WebKitGTK's build flags. The map's existing observation still stands unchanged and is in fact
reinforced: **a normal Chrome browser on Linux has full WebRTC and, with a udev rule, full WebHID.** If the pilot
consoles run Linux, the browser tier is now the *stronger* candidate, not the fallback.

---

## 8. Method and source quality

| Source | Type | Confidence | Note |
| --- | --- | --- | --- |
| [WebHID spec](https://wicg.github.io/webhid/), WICG Draft CG Report, 13 Sep 2024 | Primary — the spec text | **High** | Full text fetched and searched exhaustively. Note it is *not* a W3C standard and not on the Recommendation track. |
| Chromium `src` at [`d8a3ba21`](https://chromium.googlesource.com/chromium/src/+/d8a3ba218dabe95a746bd44f4d5747b2f3d72725) (`main`) | Primary — the implementation | **High** | ~20 files fetched and read in full. Every line cite is from this revision. |
| [`WICG/webhid/blocklist.txt`](https://github.com/WICG/webhid/blob/main/blocklist.txt) | Primary — normative by reference from spec §15 | **High** | 49 lines, read in full. |
| [systemd `rules.d/`](https://github.com/systemd/systemd/tree/main/rules.d), [Linux `drivers/hid/hidraw.c`](https://github.com/torvalds/linux/blob/master/drivers/hid/hidraw.c) | Primary — the platform | **High** | Establishes the default-deny on `/dev/hidraw*` directly rather than by folklore. |
| [mozilla/standards-positions](https://github.com/mozilla/standards-positions), [WebKit/standards-positions](https://github.com/WebKit/standards-positions) | Primary — each vendor's own record | **High** | Read the position data files and the merged PR diff, not a summary of them. |
| [Mozilla Bugzilla REST API](https://bugzilla.mozilla.org/rest/bug?summary=WebHID) | Primary — vendor bug tracker | **High** | Queried directly; confirms no implementation bug exists. |
| [chromestatus.com API](https://chromestatus.com/api/v0/features?q=WebHID) | First-party Google | **High** for shipping milestones | Used only for milestone numbers. |
| [developer.chrome.com/docs/capabilities/hid](https://developer.chrome.com/docs/capabilities/hid) | First-party vendor documentation | **High** for the udev rule, **used as confirmation only** | Every claim it supports is independently established from source above. Cited because it is where a deployment engineer will look. |
| [wpt `webhid/`](https://github.com/web-platform-tests/wpt/tree/master/webhid), `content/browser/hid/hid_browsertest.cc` | Primary — test suites | **High** | Used as *negative* evidence: the absence of visibility tests is itself the finding (§1.5). |
| [tauri-apps/tauri#14549](https://github.com/tauri-apps/tauri/issues/14549), [#13242](https://github.com/tauri-apps/tauri/issues/13242) | Primary — project issue tracker, statements by a Tauri core maintainer | **High** for what was said, **Low** as evidence of behaviour | Both are maintainer opinion and one user screenshot; neither reports a tested `requestDevice()` call. §5 treats them as such. |
| [WebView2 API reference](https://learn.microsoft.com/en-us/dotnet/api/microsoft.web.webview2.core.corewebview2permissionkind), [*Differences between Edge and WebView2*](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/browser-features) | First-party vendor API documentation | **High** for what the API surface contains | Establishes the *absence of a host-side HID hook*. Cannot establish whether the internal delegate exists — WebView2 is closed source. |
| [Electron `electron_browser_client.cc`](https://github.com/electron/electron/blob/main/shell/browser/electron_browser_client.cc), [`docs/tutorial/devices.md`](https://github.com/electron/electron/blob/main/docs/tutorial/devices.md) | Primary — an open-source Chromium embedder | **High** | Used only as a worked example of what an embedder must implement. No claim about WebView2 rests on it. |
| `openvocs/openvocs` at `main` | Primary — third-party code | **High** for what it does, **irrelevant** to the focus question | Re-read against #15's citation; the citation is accurate. |

**What I could not reach.** **WebView2 is closed source**, so §5's central question — whether it implements
`ContentBrowserClient::GetHidDelegate()` — is unanswerable from any source available to me, and no amount of
further reading will change that. GitHub's issue-search index over `MicrosoftEdge/WebView2Feedback` returned
implausibly few hits for `WebHID` and `WebUSB`, so I do **not** treat the near-absence of issues there as
evidence that nobody has hit this. The Chromium issue tracker (`issues.chromium.org`) requires JavaScript and
authentication for both its web UI and its API; three relevant issue IDs surfaced in search
([40625708](https://issues.chromium.org/issues/40625708) *"WebHID: Persistent permissions for HID devices"*,
[40145181](https://issues.chromium.org/issues/40145181) *"WebHID permission not persisted for USB device with
serial"*, [40242002](https://issues.chromium.org/issues/40242002)) but I could not read their contents, so no
claim above rests on them — §2.2 is established from `hid_chooser_context.cc` directly, which is the code those
issues are about. `chromium.googlesource.com` also refuses commit-history pages without sign-in, so **no claim
here is supported by a CL or commit message** — only by the current state of `main` at the pinned revision. That
is a real limitation: I can say what Chromium does today and cannot say when or why it started doing it.

**No secondary sources were used.** No blog posts, no Stack Overflow, no MDN prose except where it restates the
spec. Where a source is a vendor's own developer documentation rather than spec or code, it is labelled as such
in the table above.
