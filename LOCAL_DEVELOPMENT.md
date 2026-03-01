run 
cargo xtask bundle tape_delay --release
to get the vst3 plugin under target/bundled

run
cargo run --release
for quick local testing

cargo run --release -- --sample-rate 44100 --period-size 1024
