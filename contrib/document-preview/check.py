# SPDX-License-Identifier: MPL-2.0
"""Exercise the real Blitz helper without a window; no external Python packages."""
import json
import os
import pathlib
import subprocess
import time

root = pathlib.Path(__file__).resolve().parent
helper = pathlib.Path(os.environ.get('RUNYTE_PREVIEW_HELPER', root / 'target/debug/runyte-preview-helper'))

def render(text, language='html', **kwargs):
    request = dict(text=text, language=language, path=None, width=640, height=360,
                   scale=1, scroll=[0, 0], selection=None)
    request.update(kwargs)
    start = time.monotonic()
    result = subprocess.run([str(helper)], input=json.dumps(request).encode(), capture_output=True, timeout=6)
    assert result.returncode == 0, result.stderr.decode(errors='replace')
    header, pixels = result.stdout.split(b'\n', 1)
    header = json.loads(header)
    assert len(pixels) == header['raster_width'] * header['raster_height'] * 4
    return header, pixels, (time.monotonic()-start)*1000

# Known CSS geometry permits exact text hit tests independent of editor geometry.
html = '<html><body style="margin:0;background:white;font:20px/20px monospace"><p style="margin:0">Selectable text</p><a href="https://example.com">Link target</a><div style="height:2000px;width:2000px;background:#09a"></div></body></html>'
for scale, zoom in [(1, 1), (2, 1), (0.7, 1), (1, 2), (2, 2)]:
    geometry = dict(width=int(640 * scale), height=int(360 * scale), scale=scale, zoom=zoom)
    header, pixels, ms = render(html, **geometry,
                               selection=[[zoom, 12 * zoom], [250 * zoom, 12 * zoom]])
    assert header['selected'] == 'Selectable text', (scale, zoom, header)
    point = [5 * zoom, 30 * zoom]
    header, _, _ = render(html, **geometry, selection=[point, point])
    assert header['link'] == 'https://example.com', (scale, zoom, header)
header, _, _ = render(html, scroll=[180, 500])
assert header['scroll'][0] >= 180 and header['scroll'][1] >= 500, header
print('Selection and links at 1x, 2x, reduced raster scale and zoom; two-axis scrolling:', round(ms, 1), 'ms initial request')
for name, language in [('markdown.md','markdown'), ('document.html','html'), ('data.json','json'), ('broken.json','json'), ('config.yaml','yaml'), ('source.rs','rust'), ('plain.txt','text'), ('drawing.svg','svg')]:
    path = root/'fixtures'/name
    _, pixels, ms = render(path.read_text(), language, path=str(path))
    assert len(set(pixels)) > 8, name
    print(name, round(ms, 1), 'ms')
# Local raster loading has visible results; the same relative reference has no
# authority in a scratch capture. SVG's independent decoder cannot load it.
path = root/'fixtures/document.html'
html = '<body style="margin:0;background:white"><img src="image.png"></body>'
_, local, _ = render(html, path=str(path))
_, denied, _ = render(html)
assert local != denied
svg = '<svg xmlns="http://www.w3.org/2000/svg" width="160" height="120"><image href="'+str(root/'fixtures/image.png')+'" width="160" height="120"/></svg>'
_, external, _ = render(svg, 'svg')
_, empty, _ = render('<svg xmlns="http://www.w3.org/2000/svg" width="160" height="120"></svg>', 'svg')
assert external == empty, 'SVG decoder bypassed resource policy'
print('Local image admission, scratch denial and SVG external reference denial passed')

# An explicitly selected SVG fragment overrides its Markdown source language.
svg = (root/'fixtures/drawing.svg').read_text()
_, fragment, _ = render(svg, 'markdown', selection_only=True)
_, document, _ = render(svg, 'svg')
assert fragment == document
print('Selected SVG fragment dispatch passed')

_, labelled, _ = render(svg, 'svg')
_, unlabelled, _ = render(svg.replace('Static SVG preview', ''), 'svg')
assert labelled != unlabelled, 'SVG text disappeared with a generic font family'
print('SVG generic-font fallback passed')

# Admission uses canonical containment, including traversal and symlinks.
import tempfile
import shutil
with tempfile.TemporaryDirectory(prefix='runyte-preview-assets-') as temp:
    temp = pathlib.Path(temp)
    (temp/'document').mkdir()
    shutil.copyfile(root/'fixtures/image.png', temp/'outside.png')
    (temp/'document/link.png').symlink_to(temp/'outside.png')
    page = str(temp/'document/file.html')
    for reference in ['../outside.png', 'link.png']:
        _, rejected, _ = render(html.replace('image.png', reference), path=page)
        assert rejected == denied, reference
render('<html><style>body { background: white }</style><h1>Malformed <b>document')
request = dict(text='x'*131073, language='text',path=None,width=640,height=360,scale=1,scroll=[0,0],selection=None)
result = subprocess.run([str(helper)],input=json.dumps(request).encode(),capture_output=True,timeout=6)
assert result.returncode != 0 and b'budget' in result.stderr
print('Traversal/symlink denial, malformed HTML and oversized-input rejection passed')

# Large display at reduced raster scale: logical selection stays unchanged.
import math
scale = math.sqrt(3_990_000 / (3840 * 2160))
header, pixels, _ = render('<body style="margin:0;font:20px monospace">Selectable text</body>',
    width=int(3840 * scale), height=int(2160 * scale), scale=scale,
    selection=[[1, 12], [170, 12]])
assert 'Selectable' in header['selected'], header
print('Large viewport with reduced raster resolution preserves selection')

# Retained process: preserve fractional offsets, clear selection, resize, zoom,
# and replace the captured document without retaining stale text or pixels.
import statistics
server = subprocess.Popen([str(helper), '--serve'], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
def update(request):
    start = time.monotonic()
    server.stdin.write(json.dumps(request).encode() + b'\n')
    server.stdin.flush()
    header = json.loads(server.stdout.readline())
    pixels = server.stdout.read(header['raster_width'] * header['raster_height'] * 4)
    assert len(pixels) == header['raster_width'] * header['raster_height'] * 4
    return header, pixels, (time.monotonic() - start) * 1000
try:
    request = dict(text='<body style="margin:0;font:20px monospace">Selectable text<div style="height:3000px;background:#09a"></div></body>',
                   language='html', path=None, width=1080, height=800, scale=1, zoom=1, scroll=[0,0], selection=[[1,12],[250,12]])
    header, _, cold = update(request)
    assert 'Selectable text' in header['selected']
    request['selection'] = None
    times = []
    for i in range(1, 11):
        request['scroll'] = [0, i * 1.25]
        header, _, ms = update(request)
        assert header['scroll'] == request['scroll'] and header['selected'] == ''
        assert header['max_scroll'][1] > 1000
        times.append(ms)
    request.update(width=800, height=600, zoom=1.5, scroll=[0,0], selection=[[1,18],[375,18]])
    header, _, _ = update(request)
    assert 'Selectable text' in header['selected'], header
    request.update(text='New capture <literal>', language='text', zoom=1, selection=None)
    header, _, _ = update(request)
    assert header['selected'] == '' and header['scroll'] == [0,0]
    print(f'Retained renderer: cold {cold:.1f} ms; warm scroll median {statistics.median(times):.1f} ms, max {max(times):.1f} ms; fractional scrolling, zoom, resize and capture replacement passed')
    path = root / 'fixtures/markdown.md'
    request.update(text=path.read_text(), language='markdown', path=str(path), width=1080, height=800)
    update(request)
    markdown_times = []
    for i in range(1, 21):
        request['scroll'] = [0, i * 1.25]
        _, _, ms = update(request)
        markdown_times.append(ms)
    print(f'Markdown retained redraw at 1080x800: median {statistics.median(markdown_times):.1f} ms, max {max(markdown_times):.1f} ms')

finally:
    server.stdin.close()
    server.wait(timeout=6)
    assert server.returncode == 0, server.stderr.read().decode(errors='replace')


# The cached raster must contain the exact adjacent viewport, not a taller layout.
# A media query detects accidentally changing the CSS viewport height to overscan.
html = '<style>body{margin:0;background:white}div{height:2400px;background:linear-gradient(#f00,#00f)}@media(min-height:500px){div{background:#0f0}}</style><div></div>'
for display_scale in (1, 2):
    width, height = 640 * display_scale, 360 * display_scale
    cache_header, cached, _ = render(html, width=width, height=height, scale=display_scale, adjacent=True)
    assert cache_header['cacheable'] and cache_header['raster_height'] > 720 * display_scale
    assert cache_header['raster_width'] > width
    assert cache_header['origin'] == [0,0]
    _, current, _ = render(html, width=width, height=height, scale=display_scale, scroll=[0,180])
    stride = cache_header['raster_width'] * 4
    cropped = b''.join(cached[y*stride:y*stride+width*4] for y in range(180*display_scale,540*display_scale))
    assert cropped == current, 'adjacent cache changed layout or failed to preload real pixels'
for position in ('fixed', 'sticky'):
    fixed = html + '<header style="position:' + position + ';top:0;background:white">Viewport anchor</header>'
    header, _, _ = render(fixed, adjacent=True)
    assert not header['cacheable'] and header['raster_height'] == header['height'], header
print('Adjacent raster matches a later viewport exactly; CSS viewport layout and fixed/sticky fallback passed')
