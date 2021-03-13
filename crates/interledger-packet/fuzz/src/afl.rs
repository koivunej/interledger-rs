#[macro_use]
extern crate afl;

fn main() {
    fuzz!(|data: &[u8]| {
        crate::compare_packets(data);
    });
}
