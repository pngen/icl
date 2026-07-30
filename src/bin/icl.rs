fn main() {
    eprintln!(
        "{} is a library crate; no financial service connector is configured",
        env!("CARGO_PKG_NAME")
    );
    std::process::exit(1);
}
