
use nih_plug::prelude::*;
use convolution_vst_lib::TapeDelay;

fn main() {
    nih_export_standalone::<TapeDelay>();
}
