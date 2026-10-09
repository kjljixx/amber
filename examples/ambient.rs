//! Print the description of this moment that the timer starts from.
//!
//! `cargo run --offline --example ambient` uses the weather where this machine's
//! IP address seems to be; `-- Lisbon` names a place; `-- --none` gives the
//! time alone.
use std::{thread::sleep, time::Duration};

use amber::ambient::{Ambient, Place};

fn main() {
    let place = match std::env::args().nth(1).as_deref() {
        None => Some(Place::Here),
        Some("--none") => None,
        Some(name) => Some(Place::Named(name.into())),
    };
    let mut ambient = Ambient::start(place);
    loop {
        if let Some(description) = ambient.poll() {
            println!("{description}");
            return;
        }
        sleep(Duration::from_millis(25));
    }
}
