// SPDX-License-Identifier: MPL-2.0

fn main() -> anyhow::Result<()> {
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == "--native-pdf-helper")
    {
        return runyte_native::pdf::helper_main(std::env::args_os().skip(2));
    }
    runyte::cli::main(runyte::cli::Edition::Desktop, Some(&runyte_native::WINDOW))
}
