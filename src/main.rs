// SPDX-License-Identifier: MPL-2.0

#[cfg(feature = "native")]
mod native_frontend;

fn main() -> anyhow::Result<()> {
    #[cfg(feature = "native")]
    {
        if std::env::args_os()
            .nth(1)
            .is_some_and(|arg| arg == "--native-pdf-helper")
        {
            return native_frontend::pdf::helper_main(std::env::args_os().skip(2));
        }
        runyte::cli::main(
            runyte::cli::Edition::Desktop,
            Some(&native_frontend::WINDOW),
        )
    }
    #[cfg(not(feature = "native"))]
    runyte::cli::main(runyte::cli::Edition::Terminal, None)
}
