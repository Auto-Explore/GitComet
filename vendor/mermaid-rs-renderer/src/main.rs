fn main() {
    if let Err(err) = mermaid_rs_renderer::run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}
