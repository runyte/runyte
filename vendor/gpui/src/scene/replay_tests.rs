// SPDX-License-Identifier: Apache-2.0
// Runyte replay regression tests and same-binary comparison with original replay.
use super::*;
use crate::{AtlasTextureKind, TileId, px, size};
use std::{hint::black_box, time::Instant};

fn bounds(x: f32, y: f32, width: f32, height: f32) -> Bounds<ScaledPixels> {
    Bounds::new(point(x.into(), y.into()), size(width.into(), height.into()))
}

// Retain the original replay algorithm as the correctness and timing baseline.
// Both paths share the production insertion match, including fresh path IDs.
fn baseline_replay(scene: &mut Scene, range: Range<usize>, previous: &Scene) {
    for operation in &previous.paint_operations[range] {
        match operation {
            PaintOperation::Primitive(primitive) => scene.insert_primitive(primitive.clone()),
            PaintOperation::StartLayer(bounds) => scene.push_layer(*bounds),
            PaintOperation::EndLayer => scene.pop_layer(),
        }
    }
}

fn quad(area: Bounds<ScaledPixels>, mask: Bounds<ScaledPixels>) -> Quad {
    Quad {
        bounds: area,
        content_mask: ContentMask { bounds: mask },
        ..Default::default()
    }
}

fn fingerprint(scene: &Scene) -> String {
    let operations = scene
        .paint_operations
        .iter()
        .map(|operation| match operation {
            PaintOperation::StartLayer(bounds) => format!("start:{bounds:?}"),
            PaintOperation::EndLayer => "end".into(),
            PaintOperation::Primitive(primitive) => match primitive {
                Primitive::Shadow(value) => format!("shadow:{value:?}"),
                Primitive::Quad(value) => format!("quad:{value:?}"),
                Primitive::Path(value) => format!("path:{value:?}"),
                Primitive::Underline(value) => format!("underline:{value:?}"),
                Primitive::MonochromeSprite(value) => format!("mono:{value:?}"),
                Primitive::PolychromeSprite(value) => format!("poly:{value:?}"),
                Primitive::Surface(value) => format!("surface:{value:?}"),
            },
        })
        .collect::<Vec<_>>();
    format!(
        "{operations:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
        scene.shadows,
        scene.quads,
        scene.paths,
        scene.underlines,
        scene.monochrome_sprites,
        scene.polychrome_sprites,
        scene.surfaces,
        scene.layer_stack
    )
}

fn path() -> Path<ScaledPixels> {
    let mut path = Path::new(point(px(1.), px(1.)));
    path.line_to(point(px(10.), px(10.)));
    // One line only moves the current point; a second creates a triangle and
    // nonempty bounds. Fail here rather than silently testing a clipped path.
    path.line_to(point(px(10.), px(1.)));
    let mut path = path.scale(1.);
    path.content_mask = ContentMask {
        bounds: bounds(0., 0., 100., 100.),
    };
    assert!(!path.bounds.is_empty());
    assert_eq!(path.vertices.len(), 3);
    path
}

fn tile(kind: AtlasTextureKind) -> AtlasTile {
    AtlasTile {
        texture_id: AtlasTextureId { index: 0, kind },
        tile_id: TileId(0),
        padding: 0,
        bounds: Default::default(),
    }
}

fn additional_primitives(scene: &mut Scene) {
    let area = bounds(3., 3., 8., 8.);
    let mask = ContentMask {
        bounds: bounds(5., 5., 5., 5.),
    };
    scene.insert_primitive(Shadow {
        order: 0,
        blur_radius: 2f32.into(),
        bounds: area,
        corner_radii: Default::default(),
        content_mask: mask.clone(),
        color: Hsla::default(),
    });
    scene.insert_primitive(Underline {
        order: 0,
        pad: 0,
        bounds: area,
        content_mask: mask.clone(),
        color: Hsla::default(),
        thickness: 1f32.into(),
        wavy: 1,
    });
    scene.insert_primitive(MonochromeSprite {
        order: 0,
        pad: 0,
        bounds: area,
        content_mask: mask.clone(),
        color: Hsla::default(),
        tile: tile(AtlasTextureKind::Monochrome),
        transformation: TransformationMatrix::default(),
    });
    scene.insert_primitive(PolychromeSprite {
        order: 0,
        pad: 0,
        grayscale: false,
        opacity: 0.75,
        bounds: area,
        content_mask: mask.clone(),
        corner_radii: Default::default(),
        tile: tile(AtlasTextureKind::Polychrome),
    });
    // macOS surfaces require an actual CoreVideo buffer; the CPU-only Linux
    // experiment covers Surface without allocating a platform graphics object.
    #[cfg(not(target_os = "macos"))]
    scene.insert_primitive(PaintSurface {
        order: 0,
        bounds: area,
        content_mask: mask,
    });
}

#[test]
fn layered_replay_preserves_order_masks_path_ids_and_replayed_operations() {
    let full = bounds(0., 0., 100., 100.);
    let mut source = Scene::default();
    source.insert_primitive(quad(bounds(5., 5., 20., 20.), full));
    source.push_layer(full);
    source.insert_primitive(quad(bounds(10., 10., 20., 20.), bounds(15., 15., 5., 5.)));
    let visible_count = source.len();
    source.insert_primitive(quad(bounds(90., 90., 5., 5.), bounds(0., 0., 5., 5.)));
    assert_eq!(
        source.len(),
        visible_count,
        "fully clipped input is never recorded"
    );
    source.push_layer(bounds(2., 2., 20., 20.));
    source.insert_primitive(path());
    additional_primitives(&mut source);
    source.pop_layer();
    source.pop_layer();
    source.push_layer(full);
    source.pop_layer(); // Empty cached layer retains its depth effects.
    source.insert_primitive(quad(bounds(0., 0., 30., 30.), full));
    for outer_layer in [false, true] {
        let mut baseline = Scene::default();
        let mut candidate = Scene::default();
        for scene in [&mut baseline, &mut candidate] {
            scene.insert_primitive(path()); // Replayed path needs a fresh ID.
            scene.insert_primitive(quad(full, full)); // Different destination depth.
            if outer_layer {
                scene.push_layer(full);
            }
        }
        baseline_replay(&mut baseline, 0..source.len(), &source);
        candidate.replay(0..source.len(), &source);
        for scene in [&mut baseline, &mut candidate] {
            if outer_layer {
                scene.pop_layer();
            }
            scene.insert_primitive(quad(full, full)); // Probe resulting bounds-tree state.
            scene.finish();
        }
        assert_eq!(fingerprint(&baseline), fingerprint(&candidate));
        assert_eq!(candidate.paths.len(), 2);
        assert_eq!(candidate.paths[1].id, PathId(1));
        assert_eq!(candidate.shadows.len(), 1);
        assert_eq!(candidate.underlines.len(), 1);
        assert_eq!(candidate.monochrome_sprites.len(), 1);
        assert_eq!(candidate.polychrome_sprites.len(), 1);
        #[cfg(not(target_os = "macos"))]
        assert_eq!(candidate.surfaces.len(), 1);
        let mut next_baseline = Scene::default();
        let mut next_candidate = Scene::default();
        baseline_replay(&mut next_baseline, 0..baseline.len(), &baseline);
        next_candidate.replay(0..candidate.len(), &candidate);
        assert_eq!(fingerprint(&next_baseline), fingerprint(&next_candidate));
    }
}

fn text_scene(partitions: usize) -> Scene {
    let mut scene = Scene::default();
    let mask = ContentMask {
        bounds: bounds(0., 0., 1080., 800.),
    };
    for y in 0..40 {
        for partition in 0..partitions {
            let first = partition * 120 / partitions;
            let end = (partition + 1) * 120 / partitions;
            scene.push_layer(bounds(
                first as f32 * 9.,
                y as f32 * 20.,
                (end - first) as f32 * 9.,
                20.,
            ));
            for x in first..end {
                scene.insert_primitive(MonochromeSprite {
                    order: 0,
                    pad: 0,
                    bounds: bounds(x as f32 * 9., y as f32 * 20., 8., 16.),
                    content_mask: mask.clone(),
                    color: Hsla::default(),
                    tile: AtlasTile {
                        texture_id: AtlasTextureId {
                            index: 0,
                            kind: AtlasTextureKind::Monochrome,
                        },
                        tile_id: TileId(0),
                        padding: 0,
                        bounds: Default::default(),
                    },
                    transformation: TransformationMatrix::default(),
                });
            }
            scene.pop_layer();
        }
    }
    scene
}

#[test]
#[ignore = "manual CPU-only release replay timing; run serially with --nocapture"]
fn layered_replay_benchmark() {
    if cfg!(debug_assertions) {
        panic!("run this timing experiment with cargo test --release");
    }
    for partitions in [1, 2, 4] {
        let source = text_scene(partitions);
        let mut baseline = Scene::default();
        let mut candidate = Scene::default();
        baseline_replay(&mut baseline, 0..source.len(), &source);
        candidate.replay(0..source.len(), &source);
        assert_eq!(fingerprint(&baseline), fingerprint(&candidate));
        for warmup in 0..25 {
            baseline.clear();
            candidate.clear();
            baseline_replay(&mut baseline, 0..source.len(), &source);
            candidate.replay(0..source.len(), &source);
            black_box(warmup);
        }
        // Alternating ABBA groups, one binary, identical retained capacities.
        for round in 0..4 {
            for optimized in [false, true, true, false] {
                let target = if optimized {
                    &mut candidate
                } else {
                    &mut baseline
                };
                let started = Instant::now();
                for _ in 0..100 {
                    target.clear();
                    if optimized {
                        target.replay(0..source.len(), &source);
                    } else {
                        baseline_replay(target, 0..source.len(), &source);
                    }
                    black_box(&*target);
                }
                println!(
                    "scene-clear-and-replay partitions={partitions} round={round} production={optimized} iterations=100 elapsed_ns={}",
                    started.elapsed().as_nanos()
                );
            }
        }
    }
}
