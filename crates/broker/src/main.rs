#[cfg(windows)]
mod windows;

fn main() {
    #[cfg(windows)]
    if let Err(error) = windows::run() {
        eprintln!("Broker: {error}");
        std::process::exit(1);
    }
    #[cfg(not(windows))]
    eprintln!("The named-pipe Broker currently requires Windows.");
}
