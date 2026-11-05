//! Segment planning: N target frames, K chained WanAnimate2ToVideo passes of L frames each.
//! Each continued pass re-generates one frame of overlap that is dropped (as in the
//! certified canvas' "Motion Transfer" extension, `value_1 = true`).
use alloc::{format, string::String, vec::Vec};

use crate::{
    parameters::{Request, MAX_SAFE_SEED},
    Failure,
};

#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub index: u64,
    pub seed: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// Target frames N for the selected range at the requested fps.
    pub frames: u64,
    pub segments: Vec<Segment>,
    /// Frames produced before the final trim: L + (K-1)(L-1).
    pub generated_frames: u64,
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
        return Err(Failure::new("INVALID_INPUT", "sourceRange is shorter than one frame at the requested fps"));
    }
    Ok(n)
}

/// K = 1 when N <= L, else 1 + ceil((N-L)/(L-1)).
pub fn segment_count(n: u64, l: u64) -> u64 {
    if n <= l {
        return 1;
    }
    1 + (n - l).div_ceil(l - 1)
}

pub fn plan(r: &Request) -> Result<Plan, Failure> {
    let n = target_frames(r.start_ms, r.end_ms, r.fps)?;
    let l = r.advanced.segment_frames;
    let k = segment_count(n, l);
    let segments = (0..k)
        .map(|index| Segment { index, seed: (r.advanced.seed + index) % (MAX_SAFE_SEED + 1) })
        .collect();
    Ok(Plan { frames: n, segments, generated_frames: l + (k - 1) * (l - 1) })
}

/// Deterministic API node ids: shared nodes are named, segment nodes are `g{k}.{role}`.
pub fn segment_node(index: u64, role: &str) -> String {
    format!("g{index}.{role}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_cover_target_frames() {
        assert_eq!(segment_count(60, 81), 1);
        assert_eq!(segment_count(81, 81), 1);
        assert_eq!(segment_count(82, 81), 2);
        assert_eq!(segment_count(161, 81), 2);
        assert_eq!(segment_count(162, 81), 3);
        for n in 1..400 {
            let k = segment_count(n, 81);
            assert!(81 + (k - 1) * 80 >= n);
        }
    }
}