//! Foutmeldingen in het Nederlands. Windows levert OS-fouten in de taal van het systeem en met een
//! code erachter; de gebruiker krijgt een korte tekst, het pad staat al in de context.

use std::io::{self, ErrorKind};

/// Windows-codes die geen eigen `ErrorKind` hebben, of waar die te algemeen is.
fn windows_code(code: i32) -> Option<&'static str> {
    Some(match code {
        2 | 3 => "niet gevonden",
        5 => "geen toegang",
        21 => "station is niet gereed",
        32 | 33 => "in gebruik door een ander programma",
        39 | 112 => "schijf is vol",
        53 | 64 | 67 | 1231 => "netwerklocatie niet bereikbaar",
        206 => "pad is te lang",
        _ => return None,
    })
}

/// Korte Nederlandse tekst voor een OS-fout.
pub fn io_text(e: &io::Error) -> String {
    if let Some(t) = e.raw_os_error().and_then(windows_code) {
        return t.to_string();
    }
    match e.kind() {
        ErrorKind::NotFound => "niet gevonden".into(),
        ErrorKind::PermissionDenied => "geen toegang".into(),
        ErrorKind::AlreadyExists => "bestaat al".into(),
        ErrorKind::TimedOut => "duurde te lang".into(),
        ErrorKind::UnexpectedEof => "bestand is afgekapt".into(),
        ErrorKind::OutOfMemory => "onvoldoende geheugen".into(),
        _ => match e.raw_os_error() {
            Some(c) => format!("systeemfout {c}"),
            None => e.to_string(),
        },
    }
}

/// Hele foutketen als een regel, zoals `{e:#}`, met OS-fouten in het Nederlands.
pub fn nl(e: &anyhow::Error) -> String {
    e.chain()
        .map(|c| match c.downcast_ref::<io::Error>() {
            Some(io) => io_text(io),
            None => c.to_string(),
        })
        .collect::<Vec<_>>()
        .join(": ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;

    #[test]
    fn os_error_becomes_short_dutch_text_with_the_path() {
        let e: anyhow::Result<()> = Err(io::Error::from_raw_os_error(3).into());
        let e = e.context("map niet te lezen: C:/weg").unwrap_err();
        assert_eq!(nl(&e), "map niet te lezen: C:/weg: niet gevonden");
    }

    #[test]
    fn no_english_system_text_for_any_code() {
        for (code, want) in [
            (2, "niet gevonden"),
            (5, "geen toegang"),
            (32, "in gebruik door een ander programma"),
            (112, "schijf is vol"),
            (53, "netwerklocatie niet bereikbaar"),
            (9999, "systeemfout 9999"),
        ] {
            assert_eq!(io_text(&io::Error::from_raw_os_error(code)), want);
        }
    }

    #[test]
    fn plain_messages_stay_as_they_are() {
        let e = anyhow::anyhow!("geen PDF's gevonden in X");
        assert_eq!(nl(&e), "geen PDF's gevonden in X");
    }
}
