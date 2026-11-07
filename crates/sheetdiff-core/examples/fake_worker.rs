//! Nepwerker voor de test van de wachtrij: een taak "hang" blijft hangen, "stop" valt weg, de rest
//! komt terug.

use std::io::BufRead;

fn main() {
    for line in std::io::stdin().lock().lines() {
        let line = line.unwrap_or_default();
        match line.as_str() {
            "hang" => std::thread::sleep(std::time::Duration::from_secs(120)),
            "stop" => std::process::exit(3),
            _ => println!("{line}"),
        }
    }
}
