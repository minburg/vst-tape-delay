
use nih_plug::prelude::*;
use convolution_vst::TapeDelay;

fn main() {
    nih_export_standalone::<TapeDelay>();
}