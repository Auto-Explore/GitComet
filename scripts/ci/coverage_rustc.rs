use std::{env, io, process::Command};

const RESET_CALLER_PATH_REMAPS: &str = "--remap-path-prefix==";

fn main() -> io::Result<()> {
    let wrapper = env::var_os("GITCOMET_COVERAGE_RUSTC_WRAPPER")
        .ok_or_else(|| io::Error::other("missing cargo-llvm-cov compiler wrapper"))?;
    let mut compiler = Command::new(wrapper);
    compiler.args(env::args_os().skip(1));
    compiler.arg(RESET_CALLER_PATH_REMAPS);
    if cfg!(target_os = "linux") {
        compiler.args(["-C", "link-arg=-Wl,--build-id"]);
    }
    #[cfg(unix)]
    {
        Err(std::os::unix::process::CommandExt::exec(&mut compiler))
    }
    #[cfg(windows)]
    {
        std::process::exit(compiler.status()?.code().unwrap_or(1))
    }
}
