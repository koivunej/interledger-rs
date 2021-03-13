#[macro_use]
extern crate afl;

fn main() {
    fuzz!(|data: &[u8]| {
        interledger_packet_fuzz::compare_packets(data);
    });
}
