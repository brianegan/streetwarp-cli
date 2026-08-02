//! Finding the places a render jumps because Street View has no coverage.
//!
//! Google returns no panorama for a road it has never driven: a farm track, a
//! private lane, or a crossing that a routing app believes in and the world does
//! not. Those samples are dropped, and the video splices straight from one side
//! of the gap to the other. Nothing in the numbers says so. The average snap
//! error only measures the points that survived, so a missing kilometre leaves
//! it untouched.

use geo::{Distance, Geodesic, Point};

/// A stretch of route the rendered video jumps across.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gap {
    /// The frame the jump lands on.
    pub frame: usize,
    /// Total distance skipped, across every frame in this run.
    pub metres: f64,
    /// How many consecutive frames jump here. More than one means coverage is
    /// thin along a stretch rather than missing at a point.
    pub spans: usize,
    pub from: (f64, f64),
    pub to: (f64, f64),
}

impl Gap {
    /// When the jump happens in the rendered video.
    pub fn seconds(&self, frames_per_second: f64) -> f64 {
        self.frame as f64 / frames_per_second
    }
}

/// How many times the usual frame spacing a step has to be before it counts as
/// a gap, and the shortest step that ever counts.
///
/// Relative to the route's own spacing, because that varies with
/// `--frames-per-mile` and with how densely Google has driven the area. The
/// floor stops a densely sampled route reporting every minor stutter.
const GAP_MULTIPLE: f64 = 10.0;
const GAP_FLOOR_METRES: f64 = 50.0;

/// The gaps in a sequence of rendered positions, given as `(lat, lng)`.
pub fn find_gaps(points: &[(f64, f64)]) -> Vec<Gap> {
    let metres = |a: (f64, f64), b: (f64, f64)| {
        Geodesic.distance(Point::new(a.1, a.0), Point::new(b.1, b.0))
    };
    let steps = points
        .windows(2)
        .map(|pair| metres(pair[0], pair[1]))
        .collect::<Vec<_>>();
    if steps.is_empty() {
        return Vec::new();
    }

    // The median rather than the mean, because the gaps themselves are in the
    // data and would drag a mean up until they stopped looking unusual.
    let mut sorted = steps.clone();
    sorted.sort_by(f64::total_cmp);
    let usual = sorted[sorted.len() / 2];
    let threshold = (usual * GAP_MULTIPLE).max(GAP_FLOOR_METRES);

    // Runs of adjacent jumps are one thin stretch of coverage, not several
    // findings. Reported together so the warning names places to go and look at
    // rather than frame numbers.
    let mut gaps: Vec<Gap> = Vec::new();
    for (index, &step) in steps.iter().enumerate() {
        if step <= threshold {
            continue;
        }
        match gaps.last_mut() {
            Some(open) if open.frame + (open.spans - 1) == index => {
                open.metres += step;
                open.spans += 1;
                open.to = points[index + 1];
            }
            _ => gaps.push(Gap {
                frame: index + 1,
                metres: step,
                spans: 1,
                from: points[index],
                to: points[index + 1],
            }),
        }
    }
    gaps
}
