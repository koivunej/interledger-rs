#![no_main]
use libfuzzer_sys::fuzz_target;
use std::fmt::{self, Write};

use interledger_btp as curr;

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        // already fixed in HEAD
        return;
    }
    let new = curr::fuzzing::verbose_roundtrip(data);
    let old = prev::fuzzing::verbose_roundtrip(data);

    match (new, old) {
        (Ok((val, bytes)), Ok((old_val, old_bytes))) => {
            assert_eq!(bytes, old_bytes);
        }
        (Err(e), Ok((old_val, old_bytes))) => {
            if old_bytes != data {
                // good, old implementation reads any too short packet
                return;
            }
            panic!(
                "rejected {:?} but old parsed {:?} with {}; old_bytes == data: {}",
                data,
                old_val,
                e,
                old_bytes == data,
            );
        }
        (Ok((val, _)), Err(e)) => {
            panic!(
                "accepted {:?} but old failed with {} for {:?}",
                val, e, data
            );
        }
        (Err(new), Err(old)) => {
            check_display(
                &new,
                &old,
                &[(
                    "I/O Error: varuint too large",
                    "I/O Error: failed to fill whole buffer",
                )],
            );
        }
    }
});

#[allow(unused)]
fn check_slices<A: AsRef<[u8]>, B: AsRef<[u8]>>(a: &A, b: &B) {
    assert_eq!(a.as_ref(), b.as_ref());
}

#[allow(unused)]
fn check_display<A: fmt::Display, B: fmt::Display>(a: &A, b: &B, allowed: &[(&str, &str)]) {
    let mut s = String::new();

    write!(s, "{}", a).expect("new impl failed to print itself");
    let split_at = s.len();

    write!(s, "{}", b).expect("old impl failed to print itself");

    let left = &s[..split_at];
    let right = &s[split_at..];

    if left == "I/O Error: too short variable length octet string" {
        return;
    }

    for &(l, r) in allowed {
        if left == l && right == r {
            return;
        }
    }

    assert_eq!(left, right);
}
