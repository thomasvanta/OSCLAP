use nih_plug::prelude::*;

use OSCLAP::OsClap;

fn main() {
    nih_export_standalone::<OsClap>();
}
