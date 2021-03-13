use std::convert::TryFrom;
use std::fmt::{self, Write};

use interledger_packet as curr;

#[inline(always)]
pub fn compare_packets(data: &[u8]) {
    let new = curr::Packet::try_from(bytes::BytesMut::from(data));
    let old = prev::Packet::try_from(bytes04::BytesMut::from(data));

    match (new, old) {
        (Ok(x), Ok(y)) => match x {
            curr::Packet::Prepare(x) => {
                let y = match y {
                    prev::Packet::Prepare(y) => y,
                    y => unreachable!("{:?}", y),
                };

                assert_eq!(x.amount(), y.amount());
                assert_eq!(x.expires_at(), y.expires_at());
                assert_eq!(x.execution_condition(), y.execution_condition());
                check_slices(&x.destination(), &y.destination());
                assert_eq!(x.data(), y.data());
                check_slices(&x.into_data(), &y.into_data());
            }
            curr::Packet::Fulfill(x) => {
                let y = match y {
                    prev::Packet::Fulfill(y) => y,
                    y => unreachable!("{:?}", y),
                };

                assert_eq!(x.fulfillment(), y.fulfillment());
                assert_eq!(x.data(), y.data());
                check_slices(&x.into_data(), &y.into_data());
            }
            curr::Packet::Reject(x) => {
                let y = match y {
                    prev::Packet::Reject(y) => y,
                    y => unreachable!("{:?}", y),
                };

                // cannot check this because all accessors expect valid utf8 or error
                // check_display(&x.code(), &y.code());
                check_slices(&x.triggered_by().unwrap(), &y.triggered_by().unwrap());
                assert_eq!(x.message(), y.message());
                assert_eq!(x.data(), y.data());
                check_slices(&x.into_data(), &y.into_data());
            }
        },
        (Err(e), Ok(y)) => {
            panic!("should have deserialized {:?} but got {}", y, e);
        }
        (Ok(x), Err(e)) => {
            panic!("should have errored {:?} but found {:?}", e, x);
        }
        (
            Err(curr::ParseError::InvalidAddress(curr::AddressError::InvalidFormat)),
            Err(prev::ParseError::Utf8Err(_)),
        ) => {}
        (Err(x), Err(y)) => {
            let l = format!("{}", x);
            let r = format!("{}", y);

            if l != r {
                panic!("should have errored {:?} but errored {:?}", r, l);
            }
        }
    }
}

fn check_slices<A: AsRef<[u8]>, B: AsRef<[u8]>>(a: &A, b: &B) {
    assert_eq!(a.as_ref(), b.as_ref());
}

#[allow(unused)]
fn check_display<A: fmt::Display, B: fmt::Display>(a: &A, b: &B) {
    let mut s = String::new();

    write!(s, "{}", a).expect("new impl failed to print itself");
    let split_at = s.len();

    write!(s, "{}", b).expect("old impl failed to print itself");

    assert_eq!(&s[..split_at], &s[split_at..]);
}
