# Spotify and YouTube adapter guide

Runyte supplies native views, explicit browser handoffs, managed helpers and
activity leases. Authentication, service access and playback belong to an
adapter and its external backend. The local-file controller in `media.py` is the
deterministic reference application; it does not accept Spotify or YouTube URLs.
No conformance test uses a service account or contacts either service.

The service requirements below were checked against official documentation on
2026-09-08. Recheck them before distributing an adapter.

## Spotify: control an authorized device

A Spotify adapter can present catalog or library metadata in native views and
control a selected Spotify playback device. Starting or resuming playback uses
`PUT /me/player/play`, requires the playback user's Premium subscription and the
`user-modify-playback-state` scope, and can target a specific `device_id`. It does
not send decoded audio to Runyte. Spotify also warns that calls to different
Player endpoints are not guaranteed to execute in submission order; serialize
controls and reconcile observed state before reporting completion.
[Start/resume reference](https://developer.spotify.com/documentation/web-api/reference/start-a-users-playback).

Use Spotify's [Authorization Code with PKCE flow](https://developer.spotify.com/documentation/web-api/tutorials/code-pkce-flow)
for an installed application that cannot keep a client secret. Authentication
should run through an established helper and a fresh foreground browser handoff.
Store refresh tokens in a credential manager, not Runyte settings, workspace
state, view models, logs or source-controlled profiles. Treat cancellation and a
closed authentication window as ordinary incomplete authorization.

Development Mode has additional access restrictions. Its app owner needs an
active Premium subscription, and new apps are limited to five authorized users;
the migration guide describes grandfathered exceptions.
[Development Mode migration](https://developer.spotify.com/documentation/web-api/tutorials/february-2026-migration-guide).
The later July update raises the developer account's Client ID limit to 25 and
shares its Development Mode quota across those IDs. A quota-limited `429` can
include `reason: "QUOTA_EXCEEDED"`; creating another app is not a separate quota.
[July 2026 quota update](https://developer.spotify.com/blog/2026-07-23-web-api-quota-updates).

An adapter implementation should:

1. Request only the scopes needed for its metadata and playback features. Use
   `user-read-playback-state` to discover available devices and current playback,
   with explicit handling for an unavailable or restricted device.
   [Devices](https://developer.spotify.com/documentation/web-api/reference/get-a-users-available-devices),
   [playback state](https://developer.spotify.com/documentation/web-api/reference/get-information-about-the-users-current-playback).
2. Keep bounded metadata pages and stable service IDs behind native rows. Capture
   the selected device and row revision before sending a control operation.
3. Put network requests outside the SDK reader, with deadlines, bounded responses
   and backoff. Distinguish expired authorization, denied access, missing devices,
   rate limits and account quota exhaustion in retained diagnostics.
4. Report an unacknowledged control as uncertain, then inspect device state instead
   of blindly replaying it. A playback lease represents continuing work the
   adapter actually owns; an externally owned Spotify client may continue playing
   after the adapter stops. Document that ownership explicitly.

## YouTube: external playback first

The delivered baseline is a browser handoff. With the existing `handoffs.py`
example enabled as `handoffs`, run:

```text
:plugin.handoffs.browser https://www.youtube.com/
```

This opens the system browser from the current foreground invocation. It does
not provide playback state, pause, seek or audio decoding in the editor. A native
search/list adapter can offer the same explicit action for a selected video URL.

YouTube's official IFrame API controls an embedded web player. A separate web
backend can expose its documented player events and controls to an adapter, but
opening an arbitrary browser tab does not establish that channel. The backend
must account for browser autoplay blocking and wait for player readiness before
accepting controls.
[IFrame API](https://developers.google.com/youtube/iframe_api_reference).
An embedded player also has documented dimensions and player parameters; a
terminal pane is not that web surface.
[Player parameters](https://developers.google.com/youtube/player_parameters).

Keep the native view textual and label exactly which controls the external
backend exposes. Detaching Runyte must not launch another player. Do not label
browser handoff as a full in-editor player, bypass service restrictions, or add
inline video under this API. Live service access is optional validation, separate
from the local-media fixture and the plugin release gates.
