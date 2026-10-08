# SPDX-License-Identifier: MPL-2.0
"""Native-window acceptance on an isolated X11 display (e.g. Xvfb + lavapipe).

Build with --features native first. Requires libX11, libXtst, git and Poppler.
Use --output to retain window-only PNG captures; all editor state is temporary.
"""
import argparse
import ctypes as C
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
parser.add_argument("--no-system-fonts", action="store_true", help="verify the embedded fonts with an empty Fontconfig font directory list")
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
env=os.environ.copy();env.pop('WAYLAND_DISPLAY',None);env['XDG_CONFIG_HOME']=str(storage/'config');env['XDG_RUNTIME_DIR']=str(storage/'runtime');env['SHELL']='/bin/sh'
if args.no_system_fonts:
    fontconfig = storage / 'fonts.conf'
    fontconfig.write_text('<?xml version="1.0"?><!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd"><fontconfig><reset-dirs/><cachedir>' + str(storage / 'font-cache') + '</cachedir></fontconfig>')
    env['FONTCONFIG_FILE'] = str(fontconfig)
log=open(root/'window.log','w')
p=subprocess.Popen([str(binary),'--window','--ide','--config',str(storage/'config/config.yaml'),str(root/'notes.txt')],cwd=root,env=env,stdout=log,stderr=log)
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
        specials={':':('semicolon',True),' ':('space',False),'/':('slash',False),'.':('period',False),'-':('minus',False),'!':('1',True),'>':('period',True),'_':('minus',True),'\\':('backslash',False)}
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
    key('i');text('Native edit ');key('Escape');command('write')
    assert (root/'notes.txt').read_text().startswith('Native edit '),'typing or save failed'
    command('open gradient.png');time.sleep(2);image_pixels,_=screenshot('02-image');assert len(set(image_pixels)) > 100
    key('equal');key('equal');time.sleep(.5);zoomed,_=screenshot('02a-image-zoom');assert zoomed[(300*1080+200)*3:(300*1080+200)*3+3] != image_pixels[(300*1080+200)*3:(300*1080+200)*3+3],'keyboard zoom did not change image pixels'
    drag((500,350),(650,420),number=2);panned,_=screenshot('02b-image-pan');assert panned != zoomed,'middle-drag did not pan image'
    mouse(500,350);wheel(4,ctrl=True);wheel_pixels,_=screenshot('02c-pointer-zoom');assert wheel_pixels != panned,'Ctrl-wheel did not zoom'
    key('z');key('f');time.sleep(.5)
    drag((200,200),(400,350),shift=True);key('y');time.sleep(.5)
    region=clipboard(b'image/png');assert region.startswith(b'\x89PNG'),'region clipboard is not a PNG'
    assert 1 < struct.unpack('!I',region[16:20])[0] < width,'copied region did not crop image'
    screenshot('02d-region-selection')
    command('open pages.pdf');time.sleep(2);first_page,first_color=screenshot('03-pdf-first');assert first_color[2] > first_color[0] + 100, first_color
    # Left-drag across the first line selects real PDF words, not page markers.
    drag((130,155),(500,155));key('y');time.sleep(.3)
    assert clipboard(b'UTF8_STRING').decode()=='Hello Runyte','PDF selection did not copy real text'
    screenshot('03a-pdf-selection')
    key('Escape');key('v');key('l');key('y');time.sleep(.3)
    assert clipboard(b'UTF8_STRING').decode()=='Hello Runyte','keyboard PDF selection failed'
    key('Escape');key('j');time.sleep(2);second_page,second_color=screenshot('04-pdf-second');assert second_color[0] > second_color[2] + 100, second_color
    key('g');key('p');time.sleep(.3);screenshot('04a-page-picker')
    key('1');key('Return');time.sleep(.7);_,picked_color=screenshot('04b-picked-first');assert picked_color[2] > picked_color[0] + 100, picked_color
    key('g');key('p');key('Down');key('Return');time.sleep(.7);_,picked_color=screenshot('04c-picked-second');assert picked_color[0] > picked_color[2] + 100, picked_color
    key('space');time.sleep(.5);screenshot('05-key-hints');key('Escape')
    x.XResizeWindow(d, win, 1000, 700);x.XFlush(d);time.sleep(.5)
    key('w', ctrl=True); key('v'); time.sleep(.5)
    command('open notes.txt'); screenshot('06-split')
    command('terminal'); time.sleep(.5)
    text('printf native-terminal-ok > terminal-result.txt'); key('Return'); time.sleep(.5)
    assert (root/'terminal-result.txt').read_text() == 'native-terminal-ok', 'terminal input did not reach shell'
    text('cat terminal-result.txt'); key('Return'); time.sleep(.5)
    screenshot('07-terminal')
    text('exit'); key('Return'); time.sleep(.5)
    key('backslash', ctrl=True)
    # Closing with dirty text must refuse; subsequently saving and closing exits.
    command('open notes.txt'); key('i'); text('Unsaved '); key('Escape')
    close_window(); assert p.poll() is None, 'dirty close discarded changes'
    assert not (root/'notes.txt').read_text().startswith('Unsaved ')
    screenshot('08-dirty-refusal')
    command('write'); close_window(); p.wait(timeout=15)
    assert p.returncode==0,p.returncode
    print('PASS: first paint, window identity/icon, key-driven edit/save, image, PDF paging/picker/text selection, zoom/pan/region clipboard, hints, split, terminal, dirty close refusal, quit')
finally:
    if p.poll() is None:p.terminate();p.wait(timeout=15)
    log.close()
    fixture.cleanup()
