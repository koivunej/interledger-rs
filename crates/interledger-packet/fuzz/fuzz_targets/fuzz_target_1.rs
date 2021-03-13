#![no_main]
use libfuzzer_sys::fuzz_target;
use std::convert::TryFrom;
use bytes::BytesMut;

fuzz_target!(|data: &[u8]| {
    let _ = interledger_packet::Packet::try_from(BytesMut::from(data));
});
