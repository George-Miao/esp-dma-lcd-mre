# ESP DMA Hang MRE

To use, you will need to have a working esp32s3 and st7701s-driven display with parallel RGB (DPI) interface.

1. Clone this repo
2. Change GPIO configs in `src/main.rs:110` to match your hardware
3. Run `cargo run --release`
4. The screen should turn red and blue normally
5. Uncomment `src/main.rs:159`, which delays 10ms before the main loop starts
6. DMA hangs and nothing got transmitted to the screen
