// SPDX-License-Identifier: MPL-2.0
//! Print the descriptor-generated custom-key-bindings reference.
fn main() {
    print!("{}", runyte::command::discovery::guide_reference());
}
