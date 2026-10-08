use super::{reclaimed, VolumeCapacity, NOISE_FLOOR_BYTES};
use std::path::Path;

fn cap(available: u64) -> VolumeCapacity {
    VolumeCapacity {
        total_bytes: 0,
        available_bytes: available,
    }
}

#[test]
fn none_when_either_reading_is_missing() {
    assert_eq!(reclaimed(None, Some(cap(0))), None);
    assert_eq!(reclaimed(Some(cap(0)), None), None);
    assert_eq!(reclaimed(None, None), None);
}

#[test]
fn none_below_noise_floor() {
    let before = cap(1_000);
    let after = cap(1_000 + NOISE_FLOOR_BYTES - 1);
    assert_eq!(reclaimed(Some(before), Some(after)), None);
}

#[test]
fn some_at_the_noise_floor() {
    let before = cap(1_000);
    let after = cap(1_000 + NOISE_FLOOR_BYTES);
    assert_eq!(
        reclaimed(Some(before), Some(after)),
        Some(NOISE_FLOOR_BYTES)
    );
}

#[test]
fn none_when_available_space_shrinks() {
    assert_eq!(reclaimed(Some(cap(10_000)), Some(cap(9_000))), None);
}

#[test]
fn read_returns_the_home_volume_on_this_machine() {
    let capacity = VolumeCapacity::read().expect("家目录卷应可读");
    assert!(capacity.total_bytes > 0);
    assert!(capacity.available_bytes <= capacity.total_bytes);
}

#[test]
fn read_for_a_missing_path_is_none() {
    assert_eq!(
        VolumeCapacity::read_for(Path::new("/no/such/path/xyzzy")),
        None
    );
}
