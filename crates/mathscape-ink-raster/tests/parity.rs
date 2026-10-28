//! Parity with the Desktop canvas: projection (`projectInk`) and outlines
//! (`perfect-freehand@1.2.3` `getStroke`) recorded by
//! `frontend/tests/unit/ink-raster-fixture.spec.ts`.

use mathscape_domain::{InkLabDocument, InkLabStroke};
use mathscape_ink_raster::{
    InkStrokeInput, Rect, StrokeOptions, get_stroke, outline_stroke, parse_hex_color, render_png,
    render_png_within,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

const FIXTURE: &str = include_str!("../../../testdata/ink/raster/projection-outlines.json");
const TOLERANCE: f64 = 1e-6;

#[derive(Deserialize)]
struct Fixture {
    document: InkLabDocument,
    visible: Vec<VisibleStroke>,
    outlines: Vec<StrokeOutline>,
}

#[derive(Deserialize)]
struct VisibleStroke {
    id: String,
    page_id: String,
    color: String,
    points: Vec<[f64; 2]>,
}

#[derive(Deserialize)]
struct StrokeOutline {
    id: String,
    outline: Vec<[f64; 2]>,
}

fn fixture() -> Fixture {
    serde_json::from_str(FIXTURE).expect("fixture parses")
}

fn input(stroke: &InkLabStroke, opacity: f32) -> InkStrokeInput {
    InkStrokeInput {
        points: stroke
            .points
            .iter()
            .map(|point| [point.x, point.y, point.pressure.unwrap_or(0.5)])
            .collect(),
        options: StrokeOptions {
            size: stroke.brush.width,
            thinning: stroke.brush.thinning,
            smoothing: stroke.brush.smoothing,
            streamline: stroke.brush.streamline,
        },
        color: parse_hex_color(&stroke.brush.color).expect("fixture colour"),
        opacity,
    }
}

#[test]
fn projection_matches_frontend_project_ink() {
    let fixture = fixture();
    let projected = fixture
        .document
        .project_visible_strokes()
        .expect("projects");
    let ids: Vec<&str> = projected.iter().map(|stroke| stroke.id.as_str()).collect();
    let expected: Vec<&str> = fixture
        .visible
        .iter()
        .map(|stroke| stroke.id.as_str())
        .collect();
    assert_eq!(ids, expected);
    let manifest_ids = fixture.document.validate().expect("valid document");
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    let mut visible_ids: Vec<&str> = manifest_ids
        .visible_stroke_ids
        .iter()
        .map(String::as_str)
        .collect();
    visible_ids.sort_unstable();
    assert_eq!(sorted, visible_ids, "projection agrees with validate()");
    for (stroke, want) in projected.iter().zip(&fixture.visible) {
        assert_eq!(stroke.page_id, want.page_id, "{}", want.id);
        assert_eq!(stroke.brush.color, want.color, "{}", want.id);
        assert_eq!(stroke.points.len(), want.points.len(), "{}", want.id);
        for (point, [x, y]) in stroke.points.iter().zip(&want.points) {
            assert!(
                (point.x - x).abs() <= TOLERANCE,
                "{} x {} vs {x}",
                want.id,
                point.x
            );
            assert!(
                (point.y - y).abs() <= TOLERANCE,
                "{} y {} vs {y}",
                want.id,
                point.y
            );
        }
    }
}

#[test]
fn outlines_match_perfect_freehand() {
    let fixture = fixture();
    let projected = fixture
        .document
        .project_visible_strokes()
        .expect("projects");
    assert_eq!(projected.len(), fixture.outlines.len());
    let mut worst = 0.0_f64;
    for (stroke, want) in projected.iter().zip(&fixture.outlines) {
        assert_eq!(stroke.id, want.id);
        let outline = outline_stroke(&input(stroke, 1.0)).points;
        assert_eq!(
            outline.len(),
            want.outline.len(),
            "{} outline length",
            want.id
        );
        for (got, [x, y]) in outline.iter().zip(&want.outline) {
            worst = worst.max((got[0] - x).abs()).max((got[1] - y).abs());
        }
    }
    assert!(worst <= TOLERANCE, "max outline deviation {worst}");
}

#[test]
fn empty_stroke_has_no_outline() {
    let options = StrokeOptions {
        size: 4.0,
        thinning: 0.4,
        smoothing: 0.35,
        streamline: 0.15,
    };
    assert!(get_stroke(&[], options).is_empty());
}

fn page_outlines(page_id: &str) -> Vec<mathscape_ink_raster::Outline> {
    fixture()
        .document
        .project_visible_strokes()
        .expect("projects")
        .iter()
        .filter(|stroke| stroke.page_id == page_id)
        .map(|stroke| outline_stroke(&input(stroke, 1.0)))
        .collect()
}

fn page_a() -> Rect {
    Rect {
        x: 0.0,
        y: 0.0,
        width: 1200.0,
        height: 1600.0,
    }
}

#[test]
fn render_is_deterministic_and_sized_like_the_contract() {
    let outlines = page_outlines("page-a");
    let first = render_png(&outlines, &page_a(), 1024).expect("renders");
    let second = render_png(&outlines, &page_a(), 1024).expect("renders");
    assert_eq!((first.width, first.height), (768, 1024));
    assert!((first.scale - 0.64).abs() < 1e-12);
    assert_eq!(
        Sha256::digest(&first.png).as_slice(),
        Sha256::digest(&second.png).as_slice()
    );
    assert!(first.png.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert!(first.rendered > 0);
    // No text or time chunks: IHDR, IDAT(s), IEND only.
    let mut offset = 8;
    while offset < first.png.len() {
        let length = u32::from_be_bytes(first.png[offset..offset + 4].try_into().unwrap()) as usize;
        let kind = &first.png[offset + 4..offset + 8];
        assert!(matches!(kind, b"IHDR" | b"IDAT" | b"IEND"), "{kind:?}");
        offset += 12 + length;
    }
}

#[test]
fn blank_page_renders_white() {
    let blank = render_png(&[], &page_a(), 256).expect("renders");
    assert_eq!((blank.width, blank.height), (192, 256));
    assert_eq!(blank.rendered, 0);
}

#[test]
fn budget_downscales_then_gives_up() {
    let outlines = page_outlines("page-a");
    let full = render_png(&outlines, &page_a(), 1024).expect("renders");
    let budget = full.png.len() - 1;
    let fitted = render_png_within(&outlines, &page_a(), 1024, budget).expect("fits");
    let image = fitted.image.expect("smaller edge fits");
    assert_eq!(fitted.requested_edge_px, 1024);
    assert_eq!(fitted.returned_edge_px, 768);
    assert_eq!(image.height, 768);

    let hopeless = render_png_within(&outlines, &page_a(), 1024, 64).expect("runs");
    assert!(hopeless.image.is_none());
    assert_eq!(hopeless.returned_edge_px, 432);

    let floor = render_png_within(&outlines, &page_a(), 300, 64).expect("runs");
    assert!(floor.image.is_none());
    assert_eq!(floor.returned_edge_px, 256);
}
