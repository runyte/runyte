// SPDX-License-Identifier: MPL-2.0

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args_os().skip(1);
    if args.next().is_some_and(|arg| arg == "--helper") {
        use runyte_native::helper::Role;
        let role = args
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing helper role"))?;
        return match Role::parse(&role)? {
            Role::Pdf => runyte_native::pdf::helper_main(args),
            Role::Preview => {
                let mut args = args.peekable();
                let result = if args.peek().is_some_and(|arg| arg == "--serve") {
                    args.next();
                    anyhow::ensure!(args.next().is_none(), "unknown preview helper argument");
                    runyte_preview::serve(std::io::stdin().lock(), std::io::stdout().lock())
                } else {
                    runyte_preview::run_once(
                        args,
                        std::io::stdin().lock(),
                        std::io::stdout().lock(),
                    )
                };
                result.map_err(|error| anyhow::anyhow!("{error}"))
            }
        };
    }
    runyte_native::helper::initialize();
    runyte::cli::main(runyte::cli::Edition::Desktop, Some(&runyte_native::WINDOW))
}
