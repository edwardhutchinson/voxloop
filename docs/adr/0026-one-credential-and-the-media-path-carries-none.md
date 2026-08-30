# One credential, checked once; the media path carries none

Three surfaces need authenticating and they are not alike: HTTP for the admin console and configuration, the signalling WebSocket, and the mediasoup transport carrying audio. VoxLoop uses **one credential across all three, and the media transport holds none of its own.**

Signing in issues an **HttpOnly, Secure, SameSite cookie**. The signalling WebSocket authenticates from that same cookie on its upgrade request — no second token, and **no token in a query string**, which is where they end up in access logs and proxy history.

## The media transport is bound, not credentialed

A mediasoup transport can only be created **over an already-authenticated signalling channel**, and is bound to that session server-side at creation. It never carries or presents a credential.

[ADR-0008](./0008-emission-is-armed-by-the-server-and-keyed-by-the-client.md) and [ADR-0018](./0018-no-signalling-channel-means-no-emission-path.md) both make the server the enforcement point for emission, which requires the server to know with certainty which session a given RTP stream belongs to. Binding at creation supplies that for free, and leaves **nothing on the media path to steal or replay**.

A bearer token attached to the transport was the alternative. It would have been a second credential with its own lifetime and its own revocation problem, and — since [ADR-0020](./0020-the-browser-is-the-client.md) forbids the wrapper having a second code path — nowhere sensible for the Tauri build to keep it.

## The Tauri wrapper authenticates like the browser, because it is the browser

The wrapper is the same bundle in a webview. It uses the webview's own cookie store and does nothing custom. ADR-0020 states that the wrapper may only ever add an input source to the seam; a bespoke desktop token store would be precisely the second code path that decision rules out, and it would put a long-lived credential on a corporate machine's disk for no gain.

## Consequences

- **Everything is same-origin in v1.** One cookie with `SameSite` is only sufficient while the console, the API and the signalling endpoint share an origin. A cross-origin admin surface or a third-party integration calling in would break this scheme rather than bend it — and that constraint lands on [#11](https://github.com/edwardhutchinson/voxloop/issues/11)'s API, whose non-browser callers therefore cannot use cookies at all and must authenticate as service principals ([ADR-0027](./0027-a-service-principal-acts-through-a-role.md)).
- **HTTPS is mandatory, including on a LAN.** A `Secure` cookie will not be sent otherwise, so a self-hosted deployment needs a certificate story — internal CA or self-signed — and that is a deployment obligation, not a nicety.
- **Revocation is immediate and central.** Ending a sign-in invalidates the one credential, which closes the signalling channel, which drops the transports bound to it. There is no second credential that could outlive the first.
- **A stolen cookie is a full compromise**, with no second factor behind it in v1. The mitigations are the ones already chosen: HttpOnly, Secure, same-origin, and the fact that the box is only reachable inside the customer network.
