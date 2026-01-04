use nih_plug::prelude::*;

use tape_delay::TapeDelay;

fn main() {
    nih_export_standalone::<TapeDelay>();
}
