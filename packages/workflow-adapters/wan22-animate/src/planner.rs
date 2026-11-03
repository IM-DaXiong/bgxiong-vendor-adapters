//! K/G/N segmentation (plan 5.3) and deterministic per-segment derivations.
use alloc::{format, string::String, vec::Vec};

use crate::{
    parameters::{Request, MAX_SAFE_SEED},
    Failure,
};

#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    /// 0-based segment index.
    pub index: u64,
    /// First output frame this segment's window covers (offset into the prepared range).
    pub window_start: u64,
    pub seed: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// Target frames N for the selected range at the requested fps.
    pub frames: u64,
    /// Number of generation groups K.
    pub segments: Vec<Segment>,
    /// Frames the model generates G = C + (K-1)(C-O); G-N tail frames are trimmed.
    pub generated_frames: u64,
    /// Exact represented duration N*1000/fps milliseconds as a rational.
    pub duration_numerator_ms: u64,
    pub duration_denominator: u64,
}

/// N = floor(rangeMs * fps / 1000), integer arithmetic only.
pub fn target_frames(start_ms: u64, end_ms: u64, fps: u64) -> Result<u64, Failure> {
    let span = end_ms
        .checked_sub(start_ms)
        .filter(|s| *s > 0)
        .ok_or_else(|| Failure::new("INVALID_INPUT", "sourceRange is empty"))?;
    let n = span
        .checked_mul(fps)
        .ok_or_else(|| Failure::new("INVALID_INPUT", "sourceRange is too long"))?
        / 1000;
    if n == 0 {
        return Err(Failure::new(
            "INVALID_INPUT",
            "sourceRange is shorter than one frame at the requested fps",
        ));
    }
    Ok(n)
}

/// K = 1 when N <= C, else 1 + ceil((N-C)/(C-O)). Requires 0 < O < C (checked by parameters).
pub fn segment_count(n: u64, c: u64, o: u64) -> u64 {
    if n <= c {
        return 1;
    }
    let step = c - o;
    1 + (n - c).div_ceil(step)
}

/// Segment seeds: segment k uses (seed + k) wrapped into the JSON-safe range, so a
/// single user seed reproduces the whole run and no two segments share noise.
pub fn segment_seed(seed: u64, index: u64) -> u64 {
    (seed + index) % (MAX_SAFE_SEED + 1)
}

pub fn plan(r: &Request) -> Result<Plan, Failure> {
    let n = target_frames(r.start_ms, r.end_ms, r.fps)?;
    let (c, o) = (r.advanced.window_frames, r.advanced.overlap_frames);
    let k = segment_count(n, c, o);
    let segments = (0..k)
        .map(|index| Segment {
            index,
            window_start: index * (c - o),
            seed: segment_seed(r.advanced.seed, index),
        })
        .collect();
    Ok(Plan {
        frames: n,
        segments,
        generated_frames: c + (k - 1) * (c - o),
        duration_numerator_ms: n * 1000,
        duration_denominator: r.fps,
    })
}

/// Deterministic API node ids: shared nodes are named, segment nodes are `g{k}.{role}`.
pub fn segment_node(index: u64, role: &str) -> String {
    format!("g{index}.{role}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parameters::{parse, tests::sample, Mode};

    fn planned(end_ms: u64, fps: u64, c: u64, o: u64) -> Plan {
        let mut v = sample();
        v["sourceRange"]["endMs"] = end_ms.into();
        v["fps"] = fps.into();
        v["advanced"]["windowFrames"] = c.into();
        v["advanced"]["overlapFrames"] = o.into();
        plan(&parse(&v, Mode::Mix).ok().unwrap()).ok().unwrap()
    }
    fn check_cover(p: &Plan, c: u64, o: u64) {
        let k = p.segments.len() as u64;
        assert!(p.generated_frames >= p.frames, "G covers N");
        if k > 1 {
            assert!(c + (k - 2) * (c - o) < p.frames, "no extra segment");
        }
        for w in p.segments.windows(2) {
            assert_eq!(
                w[1].window_start - w[0].window_start,
                c - o,
                "positive constant progress"
            );
            assert_ne!(w[0].seed, w[1].seed);
        }
    }

    #[test]
    fn window_boundary() {
        // 77 frames at 24 fps = 3208.33ms -> 3209ms gives N=77 exactly: one segment.
        let at = planned(3209, 24, 77, 5);
        assert_eq!(
            (at.frames, at.segments.len(), at.generated_frames),
            (77, 1, 77)
        );
        // one frame more needs a second group.
        let over = planned(3250, 24, 77, 5);
        assert_eq!(
            (over.frames, over.segments.len(), over.generated_frames),
            (78, 2, 149)
        );
        check_cover(&over, 77, 5);
    }

    #[test]
    fn multi_segment() {
        // 10s at 24 fps: N=240, K = 1 + ceil(163/72) = 4, G = 77 + 3*72 = 293.
        let p = planned(10_000, 24, 77, 5);
        assert_eq!(
            (p.frames, p.segments.len(), p.generated_frames),
            (240, 4, 293)
        );
        assert_eq!(
            p.segments
                .iter()
                .map(|s| s.window_start)
                .collect::<Vec<_>>(),
            [0, 72, 144, 216]
        );
        assert_eq!(
            (p.duration_numerator_ms, p.duration_denominator),
            (240_000, 24)
        );
        check_cover(&p, 77, 5);
    }

    #[test]
    fn different_window_and_overlap() {
        // 5s at 16 fps: N=80, C=49, O=9: step 40, K = 1 + ceil(31/40) = 2, G = 89.
        let p = planned(5_000, 16, 49, 9);
        assert_eq!(
            (p.frames, p.segments.len(), p.generated_frames),
            (80, 2, 89)
        );
        check_cover(&p, 49, 9);
        let q = planned(5_000, 16, 81, 1);
        assert_eq!((q.segments.len(), q.generated_frames), (1, 81));
    }

    #[test]
    fn seeds_and_ids_are_deterministic() {
        assert_eq!(segment_seed(MAX_SAFE_SEED, 1), 0);
        assert_eq!(segment_node(2, "sampler"), "g2.sampler");
        assert_eq!(planned(10_000, 24, 77, 5), planned(10_000, 24, 77, 5));
    }
}
