# interledger-packet fuzzing

## Quickstart (cargo-fuzz)

See book for more information: https://rust-fuzz.github.io/book/cargo-fuzz.html

```
cargo install cargo-fuzz
```

Then under the interledger-packet root:

```
cargo +nightly fuzz run prepare
```

## Quickstart (afl.rs)

See book for more information: https://rust-fuzz.github.io/book/afl.html

```
cargo install afl
```

Then under `fuzz/`:

```
cargo afl build --release --bin afl
cargo afl fuzz -i afl_in -o afl_out target/release/afl
```
