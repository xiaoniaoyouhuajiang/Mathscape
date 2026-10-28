//! A line-for-line port of `perfect-freehand@1.2.3`'s `getStroke`, restricted
//! to the options the Desktop canvas uses (`canvas.ts::drawInkStroke`):
//! `simulatePressure: false`, `last: true`, no taper, round caps, identity
//! easing. Arithmetic keeps the JavaScript evaluation order (including V8's
//! `Math.hypot`) so outlines match the frontend to well below 1e-6.

/// A 2D point or vector, as perfect-freehand's `Vec2`.
pub type Vec2 = [f64; 2];

const FIXED_PI: f64 = std::f64::consts::PI + 0.0001;
const START_CAP_SEGMENTS: f64 = 13.0;
const END_CAP_SEGMENTS: f64 = 29.0;
const CORNER_CAP_SEGMENTS: f64 = 13.0;
const END_NOISE_THRESHOLD: f64 = 3.0;
const MIN_STREAMLINE_T: f64 = 0.15;
const STREAMLINE_T_RANGE: f64 = 0.85;
const MIN_RADIUS: f64 = 0.01;
const DEFAULT_FIRST_PRESSURE: f64 = 0.25;
const DEFAULT_PRESSURE: f64 = 0.5;

/// Brush parameters forwarded by `drawInkStroke` (`size` = brush width).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrokeOptions {
    pub size: f64,
    pub thinning: f64,
    pub smoothing: f64,
    pub streamline: f64,
}

#[derive(Clone, Copy, Debug)]
struct StrokePoint {
    point: Vec2,
    pressure: f64,
    vector: Vec2,
    running_length: f64,
}

/// `getStroke(points, {...options, simulatePressure: false, last: true})`.
///
/// Each input is `[x, y, pressure]`; the canvas passes `pressure ?? 0.5`.
pub fn get_stroke(points: &[[f64; 3]], options: StrokeOptions) -> Vec<Vec2> {
    outline_points(&stroke_points(points, options), options)
}

/// V8's `Math.hypot`: scale by the largest magnitude, Kahan-sum the squares.
pub fn js_hypot(a: f64, b: f64) -> f64 {
    let (a, b) = (a.abs(), b.abs());
    if a.is_infinite() || b.is_infinite() {
        return f64::INFINITY;
    }
    if a.is_nan() || b.is_nan() {
        return f64::NAN;
    }
    let max = if b > a { b } else { a };
    if max == 0.0 {
        return 0.0;
    }
    let mut sum = 0.0;
    let mut compensation = 0.0;
    for value in [a, b] {
        let n = value / max;
        let summand = n * n - compensation;
        let preliminary = sum + summand;
        compensation = (preliminary - sum) - summand;
        sum = preliminary;
    }
    sum.sqrt() * max
}

fn add(a: Vec2, b: Vec2) -> Vec2 {
    [a[0] + b[0], a[1] + b[1]]
}
fn sub(a: Vec2, b: Vec2) -> Vec2 {
    [a[0] - b[0], a[1] - b[1]]
}
fn mul(a: Vec2, n: f64) -> Vec2 {
    [a[0] * n, a[1] * n]
}
fn neg(a: Vec2) -> Vec2 {
    [-a[0], -a[1]]
}
fn per(a: Vec2) -> Vec2 {
    [a[1], -a[0]]
}
fn dpr(a: Vec2, b: Vec2) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn dist2(a: Vec2, b: Vec2) -> f64 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    dx * dx + dy * dy
}
fn dist(a: Vec2, b: Vec2) -> f64 {
    js_hypot(a[1] - b[1], a[0] - b[0])
}
fn uni(a: Vec2) -> Vec2 {
    let length = js_hypot(a[0], a[1]);
    [a[0] / length, a[1] / length]
}
fn lrp(a: Vec2, b: Vec2, t: f64) -> Vec2 {
    add(a, mul(sub(b, a), t))
}
fn prj(a: Vec2, b: Vec2, c: f64) -> Vec2 {
    add(a, mul(b, c))
}
fn rot_around(a: Vec2, c: Vec2, r: f64) -> Vec2 {
    let s = r.sin();
    let co = r.cos();
    let px = a[0] - c[0];
    let py = a[1] - c[1];
    let nx = px * co - py * s;
    let ny = px * s + py * co;
    [nx + c[0], ny + c[1]]
}

fn valid_pressure(pressure: Option<f64>) -> Option<f64> {
    pressure.filter(|value| *value >= 0.0)
}

/// `getStrokePoints` with `last: true`.
fn stroke_points(input: &[[f64; 3]], options: StrokeOptions) -> Vec<StrokePoint> {
    if input.is_empty() {
        return Vec::new();
    }
    let size = options.size;
    let t = MIN_STREAMLINE_T + (1.0 - options.streamline) * STREAMLINE_T_RANGE;
    // `None` mirrors JavaScript's missing third element.
    let mut pts: Vec<(Vec2, Option<f64>)> =
        input.iter().map(|p| ([p[0], p[1]], Some(p[2]))).collect();
    if pts.len() == 2 {
        let first = pts[0];
        let last = pts[1].0;
        pts.truncate(1);
        for i in 1..5 {
            pts.push((lrp(first.0, last, f64::from(i) / 4.0), None));
        }
    }
    if pts.len() == 1 {
        let first = pts[0];
        pts.push((add(first.0, [1.0, 1.0]), first.1));
    }
    let mut strokes = vec![StrokePoint {
        point: pts[0].0,
        pressure: valid_pressure(pts[0].1).unwrap_or(DEFAULT_FIRST_PRESSURE),
        vector: [1.0, 1.0],
        running_length: 0.0,
    }];
    let mut reached_minimum_length = false;
    let mut running_length = 0.0;
    let mut prev = strokes[0];
    let max = pts.len() - 1;
    for (i, (raw, pressure)) in pts.iter().enumerate().skip(1) {
        let point = if i == max {
            *raw
        } else {
            lrp(prev.point, *raw, t)
        };
        if prev.point[0] == point[0] && prev.point[1] == point[1] {
            continue;
        }
        let distance = dist(point, prev.point);
        running_length += distance;
        if i < max && !reached_minimum_length {
            if running_length < size {
                continue;
            }
            reached_minimum_length = true;
        }
        prev = StrokePoint {
            point,
            pressure: valid_pressure(*pressure).unwrap_or(DEFAULT_PRESSURE),
            vector: uni(sub(prev.point, point)),
            running_length,
        };
        strokes.push(prev);
    }
    strokes[0].vector = strokes.get(1).map_or([0.0, 0.0], |second| second.vector);
    strokes
}

fn stroke_radius(size: f64, thinning: f64, pressure: f64) -> f64 {
    size * (0.5 - thinning * (0.5 - pressure))
}

fn draw_dot(center: Vec2, radius: f64) -> Vec<Vec2> {
    let offset_point = add(center, [1.0, 1.0]);
    let start = prj(center, uni(per(sub(center, offset_point))), -radius);
    let mut dot = Vec::new();
    let step = 1.0 / START_CAP_SEGMENTS;
    let mut t = step;
    while t <= 1.0 {
        dot.push(rot_around(start, center, FIXED_PI * 2.0 * t));
        t += step;
    }
    dot
}

fn round_start_cap(center: Vec2, right: Vec2) -> Vec<Vec2> {
    let mut cap = Vec::new();
    let step = 1.0 / START_CAP_SEGMENTS;
    let mut t = step;
    while t <= 1.0 {
        cap.push(rot_around(right, center, FIXED_PI * t));
        t += step;
    }
    cap
}

fn round_end_cap(center: Vec2, direction: Vec2, radius: f64) -> Vec<Vec2> {
    let mut cap = Vec::new();
    let start = prj(center, direction, radius);
    let step = 1.0 / END_CAP_SEGMENTS;
    let mut t = step;
    while t < 1.0 {
        cap.push(rot_around(start, center, FIXED_PI * 3.0 * t));
        t += step;
    }
    cap
}

/// `getStrokeOutlinePoints` without simulated pressure, taper or easing.
fn outline_points(points: &[StrokePoint], options: StrokeOptions) -> Vec<Vec2> {
    let StrokeOptions {
        size,
        smoothing,
        thinning,
        ..
    } = options;
    let Some(last) = points.last() else {
        return Vec::new();
    };
    if size <= 0.0 {
        return Vec::new();
    }
    let total_length = last.running_length;
    let min_distance = (size * smoothing) * (size * smoothing);
    let mut left: Vec<Vec2> = Vec::new();
    let mut right: Vec<Vec2> = Vec::new();
    let mut radius = stroke_radius(size, thinning, last.pressure);
    let mut first_radius: Option<f64> = None;
    let mut prev_vector = points[0].vector;
    let mut prev_left = points[0].point;
    let mut prev_right = prev_left;
    let mut temp_left = prev_left;
    let mut temp_right = prev_right;
    let mut prev_sharp_corner = false;

    for (i, current) in points.iter().enumerate() {
        let StrokePoint {
            point,
            pressure,
            vector,
            running_length,
            ..
        } = *current;
        let is_last = i == points.len() - 1;
        if !is_last && total_length - running_length < END_NOISE_THRESHOLD {
            continue;
        }
        radius = if thinning != 0.0 && !thinning.is_nan() {
            stroke_radius(size, thinning, pressure)
        } else {
            size / 2.0
        };
        if first_radius.is_none() {
            first_radius = Some(radius);
        }
        // No taper: both strengths are 1.
        radius = MIN_RADIUS.max(radius);

        let next_vector = if is_last {
            vector
        } else {
            points[i + 1].vector
        };
        let next_dpr = if is_last {
            1.0
        } else {
            dpr(vector, next_vector)
        };
        let prev_dpr = dpr(vector, prev_vector);
        let sharp_corner = prev_dpr < 0.0 && !prev_sharp_corner;
        let next_sharp_corner = next_dpr < 0.0;
        if sharp_corner || next_sharp_corner {
            let offset = mul(per(prev_vector), radius);
            let step = 1.0 / CORNER_CAP_SEGMENTS;
            let mut t = 0.0;
            while t <= 1.0 {
                temp_left = rot_around(sub(point, offset), point, FIXED_PI * t);
                left.push(temp_left);
                temp_right = rot_around(add(point, offset), point, FIXED_PI * -t);
                right.push(temp_right);
                t += step;
            }
            prev_left = temp_left;
            prev_right = temp_right;
            if next_sharp_corner {
                prev_sharp_corner = true;
            }
            continue;
        }
        prev_sharp_corner = false;
        if is_last {
            let offset = mul(per(vector), radius);
            left.push(sub(point, offset));
            right.push(add(point, offset));
            continue;
        }
        let offset = mul(per(lrp(next_vector, vector, next_dpr)), radius);
        temp_left = sub(point, offset);
        if i <= 1 || dist2(prev_left, temp_left) > min_distance {
            left.push(temp_left);
            prev_left = temp_left;
        }
        temp_right = add(point, offset);
        if i <= 1 || dist2(prev_right, temp_right) > min_distance {
            right.push(temp_right);
            prev_right = temp_right;
        }
        prev_vector = vector;
    }

    let first_point = points[0].point;
    if points.len() == 1 {
        // `firstRadius || radius`: 0 and NaN fall back to the last radius.
        let dot_radius = match first_radius {
            Some(value) if value != 0.0 && !value.is_nan() => value,
            _ => radius,
        };
        return draw_dot(first_point, dot_radius);
    }
    let last_point = last.point;
    // The last point always lands on both sides, so `right` is non-empty.
    let start_cap = round_start_cap(first_point, right[0]);
    let direction = per(neg(last.vector));
    let end_cap = round_end_cap(last_point, direction, radius);
    right.reverse();
    let mut outline = left;
    outline.extend(end_cap);
    outline.extend(right);
    outline.extend(start_cap);
    outline
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hypot_matches_v8_on_simple_values() {
        assert_eq!(js_hypot(3.0, 4.0), 5.0);
        assert_eq!(js_hypot(0.0, -0.0), 0.0);
        assert_eq!(js_hypot(-2.0, 0.0), 2.0);
    }

    #[test]
    fn single_point_draws_a_closed_dot() {
        let options = StrokeOptions {
            size: 4.0,
            thinning: 0.4,
            smoothing: 0.35,
            streamline: 0.15,
        };
        let outline = get_stroke(&[[10.0, 10.0, 0.5]], options);
        assert!(outline.len() > 20);
        assert!(outline.iter().all(|p| p[0].is_finite() && p[1].is_finite()));
    }

    #[test]
    fn empty_input_has_no_outline() {
        let options = StrokeOptions {
            size: 4.0,
            thinning: 0.4,
            smoothing: 0.35,
            streamline: 0.15,
        };
        assert!(get_stroke(&[], options).is_empty());
    }
}
