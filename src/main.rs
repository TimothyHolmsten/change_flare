fn main() {
    if let Err(err) = change_flare::run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
