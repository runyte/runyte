# Browser panes

Status: proposed stub, recorded 2026-10-09. Further design is required before
implementation; the engine choice, ownership model and scope remain undecided.

## Intent

Display interactive web pages inside Runyte panes, particularly local development
servers alongside source files and online documentation. Preserve ordinary pane
management and a reliable way to return from page interaction to editor commands.
The initial target is the experimental native window. Graphical browsing in the
terminal frontend is outside the initial scope; its fallback presentation remains
to be designed.

## Candidate approach

Investigate Chromium Embedded Framework (CEF) off-screen rendering. Chromium
handles HTML, CSS, JavaScript, networking and layout; Runyte supplies viewport
geometry, forwards input, and composites browser frames inside the pane body.
Editor overlays and key hints must remain visible above the page and retain input
ownership. The existing native media surfaces provide architectural context, but
a browser is an interactive resource with its own lifecycle, not simply an image
or an editable text buffer.

Start evaluation with CEF pixel buffers uploaded to GPUI textures. Keep bounded
frame storage and avoid accumulating obsolete frames. Accelerated texture sharing
is a possible later optimization, requiring separate integration for Linux native
buffers, macOS IOSurface and Windows Direct3D resources. Engine initialization
should be on demand; ordinary editing must retain its startup and idle behavior.

System webviews through Wry remain an alternative to compare. Its Linux Wayland
GTK integration and the current GPUI window-handle limitations complicate direct
embedding. CEF avoids child-window embedding, but does not eliminate graphics
backend, event-loop or platform packaging work.

## Portability

CEF supports Linux, macOS and Windows. Most browser state, navigation and pixel
composition logic should be shareable; input translation, IME, display scaling,
event-loop integration and distribution need platform validation.

- Linux: evaluate X11 and native Wayland separately with the selected CEF version,
  including GPU/backend dependencies rather than assuming off-screen rendering
  makes the display system irrelevant.
- macOS: validate the existing native frontend as well as browser integration,
  Retina scaling, framework/helper application packaging and signing requirements.
- Windows: the branch's native frontend is not implemented. Porting it is a
  separate prerequisite, not work supplied by adding CEF.

## Questions for the design

- Where does browser ownership live: the native frontend, a dedicated service,
  or the persistent-session host? Frontend ownership could initially retain only
  view identity and URL in the host and reload on reattachment. Preserving live
  JavaScript state across detach needs a longer-lived owner. Neither behavior is
  selected yet; large frames should not casually enter the editor snapshot wire.
- How are browser resources represented, reopened, closed, split and listed in
  the Navigator/Finder? Does a second pane share a browser or open another view?
- Which commands and input modes provide navigation, address entry and a reliable
  return to editor control? Bindings must use the shared registry. Resolve focus,
  clipboard, text composition, selection, popup widgets and overlay interception.
- What is the initial policy for browser profiles, cookies, downloads, file
  uploads, permissions, external links, new windows and developer tools? Define
  the boundary between untrusted page content and editor/plugin capabilities.
- How are Chromium dependencies installed and updated, and its sandbox and helper
  processes packaged on each platform? Choose and validate the Rust binding or
  FFI boundary together with a pinned CEF distribution.
- How are hidden pages, animations, frame rates, memory, crashes and shutdown
  handled without stalling editor input or creating unnecessary idle work?

## Candidate first investigation

After design approval, prototype one browser pane displaying a local development
server and a documentation page. Exercise typing, clicking, scrolling, resizing,
display scaling, editor overlays and return to editor control. Validate Linux/X11,
Linux/Wayland and macOS early, and measure frame-copy cost and idle behavior before
committing to the rendering path. Windows requires a separate frontend milestone.

Initial discussion estimated roughly one developer-week for a one-platform proof
of concept, 4–8 weeks for usable Linux/macOS browser panes, and 2–4 months total
for broader lifecycle and packaging polish. These are unvalidated planning ranges,
not commitments; re-estimate after the prototype and ownership decisions. They do
not include the Windows native-frontend port.

## References

- [Native window architecture](../../reference/desktop-edition.md)
- [UI vocabulary](../../reference/ui-vocabulary.md)
- [Startup performance](../../reference/startup-performance.md)
- [CEF integration guide](https://github.com/chromiumembedded/cef/blob/master/docs/general_usage.md)
- [CEF rendering interface](https://github.com/chromiumembedded/cef/blob/master/include/cef_render_handler.h)
- [CEF browser/input interface](https://github.com/chromiumembedded/cef/blob/master/include/cef_browser.h)
- [Linux off-screen shared-texture backend issue](https://github.com/chromiumembedded/cef/issues/3953)
- [Wry platform integration](https://docs.rs/wry/latest/wry/)
