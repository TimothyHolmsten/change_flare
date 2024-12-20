use std::thread;

use change_flare::{cloudflare::CloudFlareApi, core::Updater};

fn main() {
    // Create updater with record
    let mut updater = Updater::<CloudFlareApi>::default();

    let t = thread::spawn(move || updater.run());
    t.join().unwrap();
}
