# SPDX-License-Identifier: MPL-2.0
"""Native-window acceptance on an isolated X11 display (e.g. Xvfb + lavapipe).

Build with --features native first. Requires libX11, libXtst, git and Poppler.
Use --output to retain window-only PNG captures; all editor state is temporary.
"""
import argparse
import ctypes as C
import json
import statistics
import os
import pathlib
import struct
import subprocess
import tempfile
import time
import zlib

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--binary", type=pathlib.Path, default=pathlib.Path("target/debug/runyte"))
parser.add_argument("--output", type=pathlib.Path)
parser.add_argument("--window-controls", action="store_true", help="exercise font settings, system clipboard, and parent editor wait")
parser.add_argument("--mux", action="store_true", help="exercise persistent window attachment and frontend handoff")
parser.add_argument("--no-system-fonts", action="store_true", help="verify the embedded fonts with an empty Fontconfig font directory list")
parser.add_argument("--paint-benchmark", action="store_true", help="measure warmed cell painting at 120x40; requires RUNYTE_NATIVE_PAINT_TIMING=1")
parser.add_argument("--latency", action="store_true", help="measure key-to-pixel latency and idle wakeups at 120x40")
parser.add_argument("--latency-keys", type=int, default=60, help="keys measured by --latency")
parser.add_argument("--max-idle-wakeups", type=float, help="with --latency, fail above this many idle context switches per second")
parser.add_argument("--paint-styles", action="store_true", help="capture styled terminal cells, including wide glyphs and decorated emoji")
parser.add_argument("--paint-reference", type=pathlib.Path, help="compare styled-cell pixels against an earlier --paint-styles output directory")
args = parser.parse_args()
binary = args.binary.resolve()
fixture = tempfile.TemporaryDirectory(prefix="runyte-native-window-")
storage = pathlib.Path(fixture.name)
root = storage / "project"
root.mkdir()
(storage / "config").mkdir()
(storage / "runtime").mkdir(mode=0o700)
(storage / "config/config.yaml").write_text("mode: ide\nlsp:\n  enable: false\n")
(root / "notes.txt").write_text("Runyte native window\nExisting keys, panes, and terminal sessions.\n")
if args.paint_benchmark:
    (root / "notes.txt").write_text(("Grid paint abcdefghijklmnopqrstuvwxyz 0123456789 == != -> " * 3 + "\n") * 100)
if args.paint_styles:
    sample = "Grid == != -> ffi  abc XYZ  \ue0b0 \uf120  界界  e\u0301  😀🌍"
    styles = [("medium", "0"), ("bold", "1"), ("italic", "3"), ("bold italic", "1;3"),
              ("dim", "2"), ("reverse", "7"), ("hidden", "8"),
              ("underline", "4"), ("crossed out", "9"), ("both", "4;9")]
    (root/'styles.ansi').write_text("\x1b[2J\x1b[H" + "".join(
        "\x1b[0m" + label.ljust(14) + "\x1b[38;2;255;80;90m\x1b[" + style + "m"
        + ("\x1b[48;2;24;24;24m" if index % 2 else "\x1b[48;2;11;33;44m")
        + content + "\x1b[0m\r\n"
        for content in [sample, sample.removesuffix("  😀🌍")]
        for index, (label, style) in enumerate(styles)) + "\x1b[14;44H")
subprocess.run(["git", "init", "-q", str(root)], check=True)

def chunk(kind, data):
    return struct.pack("!I", len(data)) + kind + data + struct.pack("!I", zlib.crc32(kind + data) & 0xffffffff)

width, height = 320, 200
rows = b"".join(b"\0" + b"".join(bytes([x * 255 // width, y * 255 // height, 140, 255]) for x in range(width)) for y in range(height))
(root / "gradient.png").write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack("!2I5B", width, height, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))
pdf = pathlib.Path(__file__).resolve().parents[1] / "src/native_frontend/tests/fixtures/two_pages.pdf"
(root / "pages.pdf").write_bytes(pdf.read_bytes())
if args.output:
    args.output.mkdir(parents=True, exist_ok=True)

x=C.CDLL('libX11.so.6'); xt=C.CDLL('libXtst.so.6')
x.XOpenDisplay.argtypes=[C.c_char_p];x.XOpenDisplay.restype=C.c_void_p
x.XDefaultRootWindow.argtypes=[C.c_void_p];x.XDefaultRootWindow.restype=C.c_ulong
x.XQueryTree.argtypes=[C.c_void_p,C.c_ulong,C.POINTER(C.c_ulong),C.POINTER(C.c_ulong),C.POINTER(C.POINTER(C.c_ulong)),C.POINTER(C.c_uint)]
x.XFetchName.argtypes=[C.c_void_p,C.c_ulong,C.POINTER(C.c_char_p)]
x.XFree.argtypes=[C.c_void_p]
x.XSetInputFocus.argtypes=[C.c_void_p,C.c_ulong,C.c_int,C.c_ulong]
x.XFlush.argtypes=[C.c_void_p]
x.XStringToKeysym.argtypes=[C.c_char_p];x.XStringToKeysym.restype=C.c_ulong
x.XKeysymToKeycode.argtypes=[C.c_void_p,C.c_ulong];x.XKeysymToKeycode.restype=C.c_uint
xt.XTestFakeKeyEvent.argtypes=[C.c_void_p,C.c_uint,C.c_int,C.c_ulong]
xt.XTestFakeMotionEvent.argtypes=[C.c_void_p,C.c_int,C.c_int,C.c_int,C.c_ulong]
xt.XTestFakeButtonEvent.argtypes=[C.c_void_p,C.c_uint,C.c_int,C.c_ulong]
x.XCreateSimpleWindow.argtypes=[C.c_void_p,C.c_ulong,C.c_int,C.c_int,C.c_uint,C.c_uint,C.c_uint,C.c_ulong,C.c_ulong]
x.XCreateSimpleWindow.restype=C.c_ulong
x.XConvertSelection.argtypes=[C.c_void_p,C.c_ulong,C.c_ulong,C.c_ulong,C.c_ulong,C.c_ulong]
x.XPending.argtypes=[C.c_void_p]
x.XNextEvent.argtypes=[C.c_void_p,C.c_void_p]
x.XDestroyWindow.argtypes=[C.c_void_p,C.c_ulong]
x.XGetWindowProperty.argtypes=[C.c_void_p,C.c_ulong,C.c_ulong,C.c_long,C.c_long,C.c_int,C.c_ulong,C.POINTER(C.c_ulong),C.POINTER(C.c_int),C.POINTER(C.c_ulong),C.POINTER(C.c_ulong),C.POINTER(C.POINTER(C.c_ubyte))]
x.XFree.argtypes=[C.c_void_p]
class Attr(C.Structure):
    _fields_=[('x',C.c_int),('y',C.c_int),('width',C.c_int),('height',C.c_int),('border_width',C.c_int),('depth',C.c_int),('visual',C.c_void_p),('root',C.c_ulong),('cls',C.c_int),('bit_gravity',C.c_int),('win_gravity',C.c_int),('backing_store',C.c_int),('backing_planes',C.c_ulong),('backing_pixel',C.c_ulong),('save_under',C.c_int),('colormap',C.c_ulong),('map_installed',C.c_int),('map_state',C.c_int),('all_event_masks',C.c_long),('your_event_mask',C.c_long),('do_not_propagate_mask',C.c_long),('override_redirect',C.c_int),('screen',C.c_void_p)]
x.XGetWindowAttributes.argtypes=[C.c_void_p,C.c_ulong,C.POINTER(Attr)]
x.XGetGeometry.argtypes=[C.c_void_p,C.c_ulong,C.POINTER(C.c_ulong),C.POINTER(C.c_int),C.POINTER(C.c_int),C.POINTER(C.c_uint),C.POINTER(C.c_uint),C.POINTER(C.c_uint),C.POINTER(C.c_uint)]
x.XGetImage.argtypes=[C.c_void_p,C.c_ulong,C.c_int,C.c_int,C.c_uint,C.c_uint,C.c_ulong,C.c_int];x.XGetImage.restype=C.c_void_p
x.XGetPixel.argtypes=[C.c_void_p,C.c_int,C.c_int];x.XGetPixel.restype=C.c_ulong
x.XDestroyImage.argtypes=[C.c_void_p]
class ClientData(C.Union):
    _fields_ = [("b", C.c_char * 20), ("s", C.c_short * 10), ("l", C.c_long * 5)]
class ClientMessage(C.Structure):
    _fields_ = [("type", C.c_int), ("serial", C.c_ulong), ("send_event", C.c_int), ("display", C.c_void_p), ("window", C.c_ulong), ("message_type", C.c_ulong), ("format", C.c_int), ("data", ClientData)]
class XEvent(C.Union):
    _fields_ = [("client", ClientMessage), ("padding", C.c_long * 24)]
x.XInternAtom.argtypes = [C.c_void_p, C.c_char_p, C.c_int]
x.XInternAtom.restype = C.c_ulong
x.XSendEvent.argtypes = [C.c_void_p, C.c_ulong, C.c_int, C.c_long, C.POINTER(XEvent)]
x.XResizeWindow.argtypes = [C.c_void_p, C.c_ulong, C.c_uint, C.c_uint]
@C.CFUNCTYPE(C.c_int, C.c_void_p, C.c_void_p)
def x_error(_display, _event):
    # Turn window disappearance into a Python assertion/cleanup, not Xlib exit.
    return 0
x.XSetErrorHandler.argtypes = [C.c_void_p]
x.XSetErrorHandler(C.cast(x_error, C.c_void_p))
d=x.XOpenDisplay(None)
assert d,'cannot connect to X11'
def children(w):
    r=C.c_ulong();p=C.c_ulong();ch=C.POINTER(C.c_ulong)();n=C.c_uint()
    x.XQueryTree(d,w,C.byref(r),C.byref(p),C.byref(ch),C.byref(n))
    values=[ch[i] for i in range(n.value)]
    if ch:x.XFree(ch)
    return values
def name(w):
    n=C.c_char_p();x.XFetchName(d,w,C.byref(n));s=n.value
    if n:x.XFree(n)
    return s
existing=set(children(x.XDefaultRootWindow(d)))
env=os.environ.copy();env.pop('WAYLAND_DISPLAY',None)
# A harness started from a Runyte terminal must not hand its parent context to the fixture.
for inherited in [key for key in env if key.startswith('RUNYTE_') and key != 'RUNYTE_NATIVE_PAINT_TIMING']:env.pop(inherited)
env['XDG_CONFIG_HOME']=str(storage/'config');env['XDG_RUNTIME_DIR']=str(storage/'runtime');env['SHELL']='/bin/sh';env['XDG_CACHE_HOME']=str(storage/'cache');env['RUNYTE_ALL_HOSTS_DIR']=str(storage/'all-hosts')
if args.paint_styles:
    # Leave the cursor in a blank cell beside an italic icon, without a shell prompt.
    env['PS1'] = ''
if args.no_system_fonts:
    fontconfig = storage / 'fonts.conf'
    fontconfig.write_text('<?xml version="1.0"?><!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd"><fontconfig><reset-dirs/><cachedir>' + str(storage / 'font-cache') + '</cachedir></fontconfig>')
    env['FONTCONFIG_FILE'] = str(fontconfig)
log=open(root/'window.log','w')
launch=[str(binary),'--window','--mux' if args.mux else '--ide','--config',str(storage/'config/config.yaml')]
if not args.mux:launch.append(str(root/'notes.txt'))
p=subprocess.Popen(launch,cwd=root,env=env,stdout=log,stderr=log)
try:
    win=None
    for _ in range(100):
        if p.poll() is not None:raise RuntimeError('editor exited '+str(p.returncode)+' '+(root/'window.log').read_text()[-3000:])
        for w in children(x.XDefaultRootWindow(d)):
            if w not in existing and name(w)==b'Runyte':
                rr=C.c_ulong();xx=C.c_int();yy=C.c_int();ww=C.c_uint();hh=C.c_uint();bb=C.c_uint();dd=C.c_uint()
                x.XGetGeometry(d,w,C.byref(rr),C.byref(xx),C.byref(yy),C.byref(ww),C.byref(hh),C.byref(bb),C.byref(dd))
                if ww.value>100 and hh.value>100:win=w;break
        if win:break
        time.sleep(.1)
    assert win,'no window found'
    for attempt in range(300):
        if p.poll() is not None:
            raise RuntimeError((root/'window.log').read_text())
        attrs=Attr();x.XGetWindowAttributes(d,win,C.byref(attrs))
        if attrs.map_state==2:break
        if attempt%50==0:print('Waiting for Runyte window to map',attrs.width,attrs.height,attrs.map_state,flush=True)
        time.sleep(.1)
    assert attrs.map_state == 2, "Runyte window did not map"
    x.XSetInputFocus(d,win,1,0);x.XFlush(d);time.sleep(1)
    def window_property(name):
        actual=C.c_ulong();fmt=C.c_int();count=C.c_ulong();after=C.c_ulong();data=C.POINTER(C.c_ubyte)()
        result=x.XGetWindowProperty(d,win,x.XInternAtom(d,name,0),0,20000,0,0,C.byref(actual),C.byref(fmt),C.byref(count),C.byref(after),C.byref(data))
        assert result == 0 and after.value == 0, name
        try:
            if fmt.value == 32:
                # Xlib expands protocol CARDINALs to native longs, which may
                # sign-extend on 64-bit hosts. The property contains 32-bit ARGB.
                return [C.cast(data,C.POINTER(C.c_ulong))[i] & 0xffffffff for i in range(count.value)]
            return C.string_at(data,count.value) if data else b''
        finally:
            if data:x.XFree(data)
    assert window_property(b'WM_CLASS').rstrip(b'\0') == b'com.runyte.Runyte\0com.runyte.Runyte', 'desktop identity'
    icon = window_property(b'_NET_WM_ICON')
    assert icon[:2] == [128,128] and len(icon) == 2 + 128*128, 'missing Runyte window icon'
    assert 0xff323232 in icon[2:] and 0xfff3f1eb in icon[2:], 'unexpected icon pixels'
    def key(sym,shift=False,ctrl=False):
        modifiers=[]
        if shift:modifiers.append(x.XKeysymToKeycode(d,x.XStringToKeysym(b'Shift_L')))
        if ctrl:modifiers.append(x.XKeysymToKeycode(d,x.XStringToKeysym(b'Control_L')))
        for m in modifiers:xt.XTestFakeKeyEvent(d,m,1,0)
        code=x.XKeysymToKeycode(d,x.XStringToKeysym(sym.encode()))
        assert code,sym
        xt.XTestFakeKeyEvent(d,code,1,0);xt.XTestFakeKeyEvent(d,code,0,0)
        for m in reversed(modifiers):xt.XTestFakeKeyEvent(d,m,0,0)
        x.XFlush(d);time.sleep(.025)
    def text(s):
        specials={'&':('7',True), '\'':('apostrophe',False), ':':('semicolon',True),' ':('space',False),'/':('slash',False),'.':('period',False),'-':('minus',False),'!':('1',True),'>':('period',True),'_':('minus',True),'\\':('backslash',False)}
        for c in s:
            sym,shift=specials.get(c,(c.lower(),c.isupper()))
            key(sym,shift)
    def command(s):text(':'+s);key('Return');time.sleep(.7)
    def close_window():
        event = XEvent()
        event.client.type = 33
        event.client.display = d
        event.client.window = win
        event.client.message_type = x.XInternAtom(d, b"WM_PROTOCOLS", 0)
        event.client.format = 32
        event.client.data.l[0] = x.XInternAtom(d, b"WM_DELETE_WINDOW", 0)
        x.XSendEvent(d, win, 0, 0, C.byref(event))
        x.XFlush(d)
        time.sleep(.5)
    def mouse(xpos, ypos):
        attr=Attr();x.XGetWindowAttributes(d,win,C.byref(attr))
        xt.XTestFakeMotionEvent(d,-1,attr.x+xpos,attr.y+ypos,0);x.XFlush(d);time.sleep(.04)
    def button(number, down):
        xt.XTestFakeButtonEvent(d,number,int(down),0);x.XFlush(d);time.sleep(.04)
    def drag(start, end, number=1, shift=False):
        shift_code=x.XKeysymToKeycode(d,x.XStringToKeysym(b'Shift_L'))
        if shift:xt.XTestFakeKeyEvent(d,shift_code,1,0)
        mouse(*start);button(number,True)
        for step in range(1,11):mouse(round(start[0]+(end[0]-start[0])*step/10),round(start[1]+(end[1]-start[1])*step/10))
        button(number,False)
        if shift:xt.XTestFakeKeyEvent(d,shift_code,0,0)
        x.XFlush(d);time.sleep(.3)
    def wheel(number, ctrl=False):
        code=x.XKeysymToKeycode(d,x.XStringToKeysym(b'Control_L'))
        if ctrl:xt.XTestFakeKeyEvent(d,code,1,0)
        button(number,True);button(number,False)
        if ctrl:xt.XTestFakeKeyEvent(d,code,0,0)
        x.XFlush(d);time.sleep(.3)
    def clipboard(mime):
        requestor=x.XCreateSimpleWindow(d,x.XDefaultRootWindow(d),0,0,1,1,0,0,0)
        prop=x.XInternAtom(d,b'RUNYTE_TEST_CLIPBOARD',0)
        x.XConvertSelection(d,x.XInternAtom(d,b'CLIPBOARD',0),x.XInternAtom(d,mime,0),prop,requestor,0);x.XFlush(d)
        result=None
        for _ in range(100):
            while x.XPending(d):
                event=XEvent();x.XNextEvent(d,C.byref(event))
                if event.client.type==31:
                    actual=C.c_ulong();fmt=C.c_int();count=C.c_ulong();after=C.c_ulong();data=C.POINTER(C.c_ubyte)()
                    x.XGetWindowProperty(d,requestor,prop,0,4*1024*1024,1,0,C.byref(actual),C.byref(fmt),C.byref(count),C.byref(after),C.byref(data))
                    if data:
                        result=C.string_at(data,count.value*(fmt.value//8));x.XFree(data)
            if result is not None:break
            time.sleep(.02)
        x.XDestroyWindow(d,requestor)
        assert result is not None,'clipboard did not offer '+repr(mime)
        return result
    def screenshot(label):
        r=C.c_ulong();xx=C.c_int();yy=C.c_int();w=C.c_uint();h=C.c_uint();border=C.c_uint();depth=C.c_uint()
        x.XGetGeometry(d,win,C.byref(r),C.byref(xx),C.byref(yy),C.byref(w),C.byref(h),C.byref(border),C.byref(depth))
        im=x.XGetImage(d,win,0,0,w.value,h.value,0xffffffffffffffff,2)
        assert im
        data=bytearray()
        for j in range(h.value):
            for i in range(w.value):
                pixel=x.XGetPixel(im,i,j);data.extend(((pixel>>16)&255,(pixel>>8)&255,pixel&255))
        x.XDestroyImage(im)
        if args.output:
            rows = b"".join(b"\0" + data[y * w.value * 3:(y + 1) * w.value * 3] for y in range(h.value))
            (args.output / (label + '.png')).write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack("!2I5B", w.value, h.value, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))
        offset = ((h.value // 2) * w.value + w.value // 2) * 3
        return data, tuple(data[offset:offset + 3])
    # First paint is required before sending input, including on software GPUs.
    for _ in range(20):
        initial, _ = screenshot('01-editor')
        if len(set(initial)) > 8:
            break
        time.sleep(.25)
    assert len(set(initial)) > 8, "first editor frame remained blank"
    if args.paint_styles:
        assert args.output and not args.mux, "style capture requires --output and standalone mode"
        command('terminal'); time.sleep(.5); text('cat styles.ansi'); key('Return'); time.sleep(.5)
        attr=Attr();x.XGetWindowAttributes(d,win,C.byref(attr))
        deadline = time.monotonic() + 15
        while True:
            pixels, _ = screenshot('styled-cells')
            last_row = b''.join(pixels[(y*attr.width+135)*3:(y*attr.width+550)*3] for y in range(400, 420))
            if sum(last_row[i:i+3] == b'\xff\x50\x5a' for i in range(0, len(last_row), 3)) > 100:
                break
            assert time.monotonic() < deadline, 'styled terminal output was not painted'
            time.sleep(.1)
        # Only fixed terminal rows: exclude dynamic titles, prompt and status text.
        crop = b''.join(pixels[(y*attr.width+9)*3:(y*attr.width+1000)*3] for y in range(20, 420))
        (args.output/'styled-cells.rgb').write_bytes(crop)
        if args.paint_reference:
            assert crop == (args.paint_reference/'styled-cells.rgb').read_bytes(), 'styled-cell pixels changed'
        text('exit'); key('Return'); time.sleep(.5); key('backslash',ctrl=True)
        close_window(); p.wait(timeout=15); assert p.returncode == 0
        print('PASS: styled-cell capture' + (' matches reference' if args.paint_reference else ''))
        raise SystemExit(0)
    if args.paint_benchmark:
        assert not args.mux, "benchmark uses a standalone window"
        x.XResizeWindow(d, win, 1080, 800); x.XFlush(d); time.sleep(.5)
        key('i'); text('warmup'); time.sleep(.5)
        offset = (root/'window.log').stat().st_size
        text('abcdefghijklmnopqrstuvwxyz' * 4); time.sleep(.5)
        samples = []
        for line in (root/'window.log').read_bytes()[offset:].decode().splitlines():
            if line.startswith('native-paint 120x40 '):
                samples.append(int(line.split()[-1].removesuffix('us')))
        assert samples, "enable RUNYTE_NATIVE_PAINT_TIMING=1"
        result = {"columns": 120, "rows": 40, "samples_us": samples,
                  "median_us": statistics.median(samples), "min_us": min(samples), "max_us": max(samples)}
        print(json.dumps(result), flush=True)
        if args.output:
            (args.output/'paint-timings.json').write_text(json.dumps(result, indent=2) + '\n')
        screenshot('benchmark-grid')
        key('Escape'); command('write')
        assert (root/'notes.txt').read_text().startswith('warmup' + 'abcdefghijklmnopqrstuvwxyz' * 4), "benchmark dropped typing"
        close_window(); p.wait(timeout=15); assert p.returncode == 0
        raise SystemExit(0)
    if args.latency:
        # Key press to the first changed pixel of the edited row, read back
        # from the X server. Keys are spaced so each one starts from an idle
        # window; this measures the latency of an isolated keystroke.
        assert not args.mux, "latency uses a standalone window"
        class XImage(C.Structure):
            _fields_ = [('width', C.c_int), ('height', C.c_int), ('xoffset', C.c_int), ('format', C.c_int),
                        ('data', C.c_void_p), ('byte_order', C.c_int), ('bitmap_unit', C.c_int),
                        ('bitmap_bit_order', C.c_int), ('bitmap_pad', C.c_int), ('depth', C.c_int),
                        ('bytes_per_line', C.c_int), ('bits_per_pixel', C.c_int)]
        def region(rx, ry, rw, rh):
            im = x.XGetImage(d, win, rx, ry, rw, rh, 0xffffffffffffffff, 2)
            assert im
            image = XImage.from_address(im)
            data = C.string_at(image.data, image.bytes_per_line * image.height)
            x.XDestroyImage(im)
            return data
        def context_switches():
            # Live threads only; the editor's threads persist while it is idle.
            total = 0
            for task in pathlib.Path(f'/proc/{p.pid}/task').iterdir():
                try:
                    for line in (task / 'status').read_text().splitlines():
                        if line.startswith(('voluntary_ctxt_switches', 'nonvoluntary_ctxt_switches')):
                            total += int(line.split()[1])
                except OSError:
                    pass
            return total
        x.XResizeWindow(d, win, 1080, 800); x.XFlush(d); time.sleep(.5)
        key('g'); key('g'); key('i'); time.sleep(.5)
        # Watch the cell row whose pixels changed most: the edited text line,
        # not a title or status marker that changes with it.
        before = region(0, 0, 1080, 800); key('x'); time.sleep(.5); after = region(0, 0, 1080, 800)
        changed = [sum(a != b for a, b in zip(before[y * 4320:(y + 1) * 4320], after[y * 4320:(y + 1) * 4320]))
                   for y in range(800)]
        cell_rows = [sum(changed[row * 20:(row + 1) * 20]) for row in range(40)]
        assert max(cell_rows), "typing did not change the window"
        band = (0, cell_rows.index(max(cell_rows)) * 20, 320, 20)
        key('BackSpace'); time.sleep(.5)
        samples = []
        for index in range(args.latency_keys):
            reference = region(*band)
            sym = 'x' if index % 2 == 0 else 'BackSpace'
            code = x.XKeysymToKeycode(d, x.XStringToKeysym(sym.encode()))
            started = time.perf_counter()
            xt.XTestFakeKeyEvent(d, code, 1, 0); xt.XTestFakeKeyEvent(d, code, 0, 0); x.XFlush(d)
            while region(*band) == reference:
                assert time.perf_counter() - started < 1, "key did not reach the screen"
            samples.append((time.perf_counter() - started) * 1000)
            time.sleep(.12 + (index % 5) * .01)
        time.sleep(1)
        switches = context_switches(); time.sleep(5)
        idle = (context_switches() - switches) / 5
        ordered = sorted(samples)
        result = {"columns": 120, "rows": 40, "keys": len(samples),
                  "median_ms": round(statistics.median(samples), 2),
                  "p90_ms": round(ordered[len(ordered) * 9 // 10], 2),
                  "max_ms": round(max(samples), 2),
                  "idle_context_switches_per_second": idle}
        print(json.dumps(result), flush=True)
        if args.output:
            (args.output/'latency.json').write_text(json.dumps(result, indent=2) + '\n')
        if args.max_idle_wakeups is not None:
            assert idle <= args.max_idle_wakeups, f"idle window woke {idle}/s"
        key('Escape'); command('write')
        assert (root/'notes.txt').read_text() == "Runyte native window\nExisting keys, panes, and terminal sessions.\n", "latency keys were not applied in order"
        close_window(); p.wait(timeout=15); assert p.returncode == 0
        raise SystemExit(0)
    if args.window_controls:
        command('open notes.txt')
        key('percent', shift=True); key('c', shift=True, ctrl=True); time.sleep(.3)
        copied = clipboard(b'UTF8_STRING').decode()
        assert copied == (root/'notes.txt').read_text(), 'Ctrl+Shift+C did not publish editor selection'
        command('open pasted.txt'); key('i'); key('v', shift=True, ctrl=True); key('Escape'); command('write')
        assert (root/'pasted.txt').read_text() == copied, 'Ctrl+Shift+V did not paste copied text'
        key('percent', shift=True); text(' cy'); time.sleep(.3)
        assert clipboard(b'UTF8_STRING').decode() == copied, 'Space c y did not publish text'
        # Plain Ctrl-v must use the same native owner, including image probing.
        command('open pasted-again.txt'); key('i'); key('v', ctrl=True); key('Escape'); command('write')
        assert (root/'pasted-again.txt').read_text() == copied, 'Ctrl-v could not read Space c y clipboard'
        command('open gradient.png'); time.sleep(1)
        drag((200,200),(400,350),shift=True); key('c',shift=True,ctrl=True); time.sleep(.3)
        assert clipboard(b'image/png').startswith(b'\x89PNG'), 'Ctrl+Shift+C did not copy image region'
        command('open image-paste.md'); key('i'); key('v',ctrl=True); key('Escape'); command('write')
        assert '[Image 1](' in (root/'image-paste.md').read_text(), 'native clipboard image did not paste as a reference'
        assert list((root/'.runyte/cache/images').glob('*.png')), 'image paste did not retain PNG bytes'
        (root/'paste-command.txt').write_text('touch clipboard-terminal-ok')
        command('open paste-command.txt'); key('percent', shift=True); text(' cy')
        command('terminal'); time.sleep(.4); key('v', shift=True, ctrl=True); key('Return'); time.sleep(.4)
        assert (root/'clipboard-terminal-ok').exists(), 'clipboard paste did not reach terminal child'
        def terminal_size(name):
            text('stty size > '+name); key('Return'); time.sleep(.4)
            return tuple(map(int, (root/name).read_text().split()))
        initial = terminal_size('font-initial')
        key('equal', shift=True, ctrl=True); time.sleep(.5)
        enlarged = terminal_size('font-enlarged')
        screenshot('font-enlarged')
        assert all(a > b for a,b in zip(initial,enlarged)), (initial,enlarged)
        key('minus', ctrl=True); time.sleep(.5)
        assert terminal_size('font-restored') == initial
        assert 'font_size' not in (storage/'config/config.yaml').read_text(), 'temporary zoom changed config'
        if args.mux:
            (root/'wait.txt').write_text('Original text\n')
            text(str(binary)+' --wait wait.txt && touch wait-done'); key('Return'); time.sleep(1)
            key('i'); text('Parent edit '); key('Escape'); command('wq'); time.sleep(.5)
            assert (root/'wait.txt').read_text() == 'Parent edit Original text\n', 'wait did not open in parent window'
            assert (root/'wait-done').exists(), 'wait client did not finish successfully'
        key('equal', ctrl=True); time.sleep(.5)
        text('exit'); key('Return'); time.sleep(.4)
        close_window(); p.wait(timeout=15); assert p.returncode == 0
        # A newly opened window reads config even when its mux host already exists.
        (storage/'config/config.yaml').write_text('mode: ide\nlsp:\n  enable: false\neditor:\n  font_size: 21\n')
        existing=set(children(x.XDefaultRootWindow(d)))
        p=subprocess.Popen(launch,cwd=root,env=env,stdout=log,stderr=log)
        win=None
        for _ in range(150):
            for w in children(x.XDefaultRootWindow(d)):
                if w not in existing and name(w)==b'Runyte':
                    attr=Attr();x.XGetWindowAttributes(d,w,C.byref(attr))
                    if attr.map_state==2:win=w;break
            if win:break
            assert p.poll() is None,(root/'window.log').read_text()[-3000:]
            time.sleep(.1)
        assert win, 'restart did not create a window'
        x.XSetInputFocus(d,win,1,0);x.XFlush(d);time.sleep(1)
        command('terminal'); time.sleep(.4)
        configured=terminal_size('font-configured')
        screenshot('font-configured')
        assert all(a > b for a,b in zip(enlarged,configured)), (enlarged,configured)
        text('exit'); key('Return'); time.sleep(.4); close_window(); p.wait(timeout=15)
        assert p.returncode == 0
        print('PASS: native clipboard copy/paste, terminal paste, font resizing, restart config' + (', parent editor wait' if args.mux else ''))
        raise SystemExit(0)
    if args.mux:command('open notes.txt')
    key('i');text('Native edit ');key('Escape');command('write')
    assert (root/'notes.txt').read_text().startswith('Native edit '),'typing or save failed'
    command('open gradient.png');time.sleep(2);image_pixels,_=screenshot('02-image');assert len(set(image_pixels)) > 100
    key('equal');key('equal');time.sleep(.5);zoomed,_=screenshot('02a-image-zoom');assert zoomed[(300*1080+200)*3:(300*1080+200)*3+3] != image_pixels[(300*1080+200)*3:(300*1080+200)*3+3],'keyboard zoom did not change image pixels'
    for direction in ['h','j','k','l']:
        before_pan,_=screenshot('02a-before-'+direction)
        key(direction);time.sleep(.2);after_pan,_=screenshot('02a-pan-'+direction)
        assert after_pan != before_pan, 'zoomed image did not pan with '+direction
    drag((500,350),(650,420),number=2);panned,_=screenshot('02b-image-pan');assert panned != zoomed,'middle-drag did not pan image'
    mouse(500,350);wheel(4,ctrl=True);wheel_pixels,_=screenshot('02c-pointer-zoom');assert wheel_pixels != panned,'Ctrl-wheel did not zoom'
    key('z');key('f');time.sleep(.5)
    drag((200,200),(400,350),shift=True);key('y');time.sleep(.5)
    region=clipboard(b'image/png');assert region.startswith(b'\x89PNG'),'region clipboard is not a PNG'
    assert 1 < struct.unpack('!I',region[16:20])[0] < width,'copied region did not crop image'
    screenshot('02d-region-selection')
    key('equal');time.sleep(.2)
    key('g');time.sleep(.4);hint_image,_=screenshot('02e-image-hints');assert len(set(hint_image)) > 100, 'hints hid image'
    key('Escape');time.sleep(.2)
    key('Escape');time.sleep(.2)  # Fit before clearing the selected region.
    key('y');time.sleep(.2)
    assert clipboard(b'image/png') == region, 'image fit Escape cleared its region selection'
    key('Escape');time.sleep(.2)  # Clear the selected region.
    key('Escape');time.sleep(.3);screenshot('02f-image-explorer')
    key('Return');time.sleep(.4);_,reopened_image=screenshot('02g-image-reopened')
    command('open pages.pdf');time.sleep(2);first_page,first_color=screenshot('03-pdf-first');assert first_color[2] > first_color[0] + 100, first_color
    # Left-drag across the first line selects real PDF words, not page markers.
    drag((130,155),(500,155));key('y');time.sleep(.3)
    assert clipboard(b'UTF8_STRING').decode()=='Hello Runyte','PDF selection did not copy real text'
    screenshot('03a-pdf-selection')
    key('equal');time.sleep(.3);zoomed_pdf,_=screenshot('03b-pdf-zoom')
    key('j');time.sleep(.3);panned_pdf,panned_color=screenshot('03c-pdf-pan')
    assert panned_pdf != zoomed_pdf, 'j did not pan the zoomed PDF'
    assert panned_color[2] > panned_color[0] + 100, 'j changed the zoomed PDF page'
    key('k');time.sleep(.2)
    key('g');key('Escape');time.sleep(.2)
    _,still_zoomed_color=screenshot('03d-pdf-prefix-dismissed')
    assert still_zoomed_color[2] > still_zoomed_color[0] + 100, 'prefix Escape left the PDF'
    key('Escape');time.sleep(.3);_,fitted_color=screenshot('03e-pdf-fit-with-selection')
    assert fitted_color[2] > fitted_color[0] + 100, 'zoom Escape left the PDF'
    # The first Escape fitted the page without clearing the selected words.
    key('y');time.sleep(.2)
    assert clipboard(b'UTF8_STRING').decode()=='Hello Runyte', 'fit Escape cleared the PDF selection'
    key('Escape');key('v');key('l');key('y');time.sleep(.3)
    assert clipboard(b'UTF8_STRING').decode()=='Hello Runyte','keyboard PDF selection failed'
    key('Escape');key('j');time.sleep(.15);second_page,second_color=screenshot('04-pdf-second');assert second_color[0] > second_color[2] + 100, second_color
    key('equal')
    for previous,next_page in [('p','n'),('b','f'),('u','d')]:
        key(previous,ctrl=True);time.sleep(.3);_,page_color=screenshot('04-shortcut-'+previous)
        assert page_color[2] > page_color[0] + 100, 'previous-page shortcut failed at zoom: '+previous
        key(next_page,ctrl=True);time.sleep(.3);_,page_color=screenshot('04-shortcut-'+next_page)
        assert page_color[0] > page_color[2] + 100, 'next-page shortcut failed at zoom: '+next_page
    key('Escape');time.sleep(.2)  # Return to fit before testing the page-buffer step.
    key('g');time.sleep(.4);_,hint_color=screenshot('04a-pdf-hints');assert hint_color[0] > hint_color[2] + 100, 'hints hid PDF'
    mouse(80,40);button(1,True);button(1,False)
    mouse(200,650);button(1,True);button(1,False)  # Hints must not select hidden page rows.
    key('Escape');time.sleep(.2)  # Cancel the key prefix, stay in preview.
    _,after_hint_click=screenshot('04a-after-hint-click');assert after_hint_click[0] > after_hint_click[2] + 100, 'hint click changed the PDF page'
    key('Escape');time.sleep(.4);_,rows_color=screenshot('04b-page-buffer');assert rows_color[0] < 100, 'Escape did not show page rows'
    key('g');key('g');key('Return');time.sleep(.4);_,picked_color=screenshot('04c-picked-first');assert picked_color[2] > picked_color[0] + 100, picked_color
    key('Escape');time.sleep(.3);key('j');key('Return');time.sleep(.4);_,picked_color=screenshot('04d-picked-second');assert picked_color[0] > picked_color[2] + 100, picked_color
    key('Escape');time.sleep(.3);key('Escape');time.sleep(.3);screenshot('04e-pdf-explorer')
    key('Return');time.sleep(.4);_,reopened_color=screenshot('04f-pdf-reopened');assert reopened_color[0] > reopened_color[2] + 100, 'PDF page was not retained'
    key('space');time.sleep(.5);screenshot('05-key-hints');key('Escape')
    x.XResizeWindow(d, win, 1000, 700);x.XFlush(d);time.sleep(.5)
    key('w', ctrl=True); key('v'); time.sleep(.5)
    command('open notes.txt'); screenshot('06-split')
    command('terminal'); time.sleep(.5)
    text('printf native-terminal-ok > terminal-result.txt'); key('Return'); time.sleep(.5)
    assert (root/'terminal-result.txt').read_text() == 'native-terminal-ok', 'terminal input did not reach shell'
    text('cat terminal-result.txt'); key('Return'); time.sleep(.5)
    screenshot('07-terminal')
    if args.mux:
        key('backslash', ctrl=True)
        command('open notes.txt'); key('i'); text('Unsaved '); key('Escape')
        command('quit-all'); assert p.poll() is None, 'quit discarded protected session state'
        destination = storage / 'other'
        destination.mkdir()
        subprocess.run(['git','init','-q',str(destination)],check=True,env=env)
        command('session-attach '+str(destination))
        key('w',ctrl=True);key('a');time.sleep(.8)  # Return to the protected source.
        key('w',ctrl=True);key('a');time.sleep(.8)  # Back to the clean destination.
        command('quit-all');assert p.poll() is None, 'quit did not reuse the window for the source session'
        close_window();p.wait(timeout=15);assert p.returncode == 0
        assert not (root/'notes.txt').read_text().startswith('Unsaved ')
        # A real terminal client takes over without a second attachment loop.
        import pty, select, fcntl, termios
        master, slave = pty.openpty()
        fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',40,120,0,0))
        terminal = subprocess.Popen([str(binary),'--mux','--config',str(storage/'config/config.yaml')],cwd=root,env=env,stdin=slave,stdout=slave,stderr=slave,start_new_session=True)
        os.close(slave)
        try:
            output = bytearray();deadline=time.monotonic()+15
            while b'MEDIA UNSUPPORTED IN THE TERMINAL MODE' not in output and time.monotonic()<deadline:
                if select.select([master],[],[],.1)[0]:output.extend(os.read(master,65536))
            assert b'MEDIA UNSUPPORTED IN THE TERMINAL MODE' in output, output[-2000:]
            os.write(master,b':detach\r');terminal.wait(timeout=15);assert terminal.returncode==0
        finally:
            if terminal.poll() is None:terminal.terminate();terminal.wait(timeout=15)
            os.close(master)
        existing=set(children(x.XDefaultRootWindow(d)))
        p=subprocess.Popen(launch,cwd=root,env=env,stdout=log,stderr=log)
        win=None
        for _ in range(150):
            for w in children(x.XDefaultRootWindow(d)):
                if w not in existing and name(w)==b'Runyte':
                    attr=Attr();x.XGetWindowAttributes(d,w,C.byref(attr))
                    if attr.map_state==2:win=w;break
            if win:break
            assert p.poll() is None,(root/'window.log').read_text()[-3000:]
            time.sleep(.1)
        assert win,'reattachment did not create a window'
        x.XSetInputFocus(d,win,1,0);x.XFlush(d);time.sleep(1)
        screenshot('08-reattached');command('write')
        assert (root/'notes.txt').read_text().startswith('Unsaved Native edit '), 'handoff lost unsaved text or quit fallback selected the wrong session'
        close_window();p.wait(timeout=15);assert p.returncode==0
    else:
        text('exit'); key('Return'); time.sleep(.5)
        key('backslash', ctrl=True)
        # Closing with dirty text must refuse; subsequently saving and closing exits.
        command('open notes.txt'); key('i'); text('Unsaved '); key('Escape')
        close_window(); assert p.poll() is None, 'dirty close discarded changes'
        assert not (root/'notes.txt').read_text().startswith('Unsaved ')
        screenshot('08-dirty-refusal')
        command('write'); close_window(); p.wait(timeout=15)
        assert p.returncode==0,p.returncode
    print('PASS: first paint, window identity/icon, key-driven edit/save, image, PDF paging/page-buffer/back navigation/text selection, zoom/pan/region clipboard, hints, split, terminal; ' + ('persistent detach, terminal/window handoff, unsaved state, live child, switching, quit fallback' if args.mux else 'dirty close refusal, quit'))
finally:
    if p.poll() is None:p.terminate();p.wait(timeout=15)
    if args.mux:
        for project in [root, storage/'other']:
            subprocess.run([str(binary),'--session-stop','--force',str(project)],cwd=root,env=env,stdout=log,stderr=log,timeout=15)
    log.close()
    fixture.cleanup()
