//! The records kept on the SD card before the store, a file a day, e.g. /sd/capture/tesla/2026-10-29.jsonl, each record
//! a line of JSON in the form the app saved, which are read once, to convert them into the store, see db.rs, and left
//! where they are.

use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;

use anyhow::Context;
use chrono::{DateTime, Utc};
use serde::Deserialize;

/// How many days of records are read when since is not given, see write_since.
const DAYS_WITHOUT_SINCE: i64 = 2;

/// Writes, with write, the records in the files in dir of times after since, oldest first, each a line, as they are
/// in the files, or of the last DAYS_WITHOUT_SINCE days if since is None, up to limit, if there is one, returning how
/// many, and whether there are more, which the app asks for next, since the last it was given. The files are read a
/// line at a time, as a few days of records would not fit in the RAM, and the first day's from about where since is,
/// see start_after, as reading a whole day's from the card to find the last few minutes' takes a long time.
pub fn write_since(
    dir: &Path,
    since: Option<DateTime<Utc>>,
    limit: Option<usize>,
    write: &mut dyn FnMut(&[u8]) -> anyhow::Result<()>,
) -> anyhow::Result<(usize, bool)> {
    let since = since.unwrap_or_else(|| Utc::now() - chrono::TimeDelta::days(DAYS_WITHOUT_SINCE));
    let first_day = since.format("%Y-%m-%d").to_string();
    let mut days: Vec<String> = match std::fs::read_dir(dir).map_err(|error| (error.kind(), error)) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
            .filter_map(|name| name.strip_suffix(".jsonl").map(String::from))
            .filter(|day| *day >= first_day)
            .collect(),
        // None yet.
        Err((std::io::ErrorKind::NotFound, _)) => Vec::new(),
        Err((_, error)) => return Err(anyhow::Error::from(error).context(format!("listing {}", dir.display()))),
    };
    days.sort();

    let mut count = 0;
    for day in days {
        let path = dir.join(format!("{day}.jsonl"));
        let mut file = File::open(&path).with_context(|| format!("opening {}", path.display()))?;
        if day == first_day {
            let start = start_after(&mut file, since).with_context(|| format!("searching {}", path.display()))?;
            file.seek(SeekFrom::Start(start)).with_context(|| format!("reading {}", path.display()))?;
        }
        for line in BufReader::with_capacity(READ_BUFFER, file).split(b'\n') {
            let line = line.with_context(|| format!("reading {}", path.display()))?;
            // A line being written as it is read, or cut short by a power cut, is left out.
            if line_time(&line).is_some_and(|time| time > since) {
                if limit.is_some_and(|limit| count >= limit) {
                    return Ok((count, true));
                }
                write(&line)?;
                count += 1;
            }
        }
    }
    Ok((count, false))
}

/// How much of a file is read from the card at a time.
const READ_BUFFER: usize = 4096;

/// When the search for since stops, see start_after, after which the lines are read one by one, a few records.
const SEARCH_ENOUGH: u64 = 8 * 1024;

/// Where in file, a day's records, which are in time order, as they are added as they are taken, reading from which
/// gives all those after since: the start of a line of a time before it, or of the file, found by halving the part
/// of the file it could be in, and reading a line from the middle, a few reads, rather than reading the whole file.
fn start_after(file: &mut File, since: DateTime<Utc>) -> anyhow::Result<u64> {
    let (mut low, mut high) = (0, file.metadata()?.len());
    while high - low > SEARCH_ENOUGH {
        let middle = low + (high - low) / 2;
        file.seek(SeekFrom::Start(middle))?;
        let mut reader = BufReader::with_capacity(READ_BUFFER, &mut *file);
        // The rest of the line the middle is in.
        let mut line = Vec::new();
        let start = middle + reader.read_until(b'\n', &mut line)? as u64;
        line.clear();
        reader.read_until(b'\n', &mut line)?;
        match line_time(&line) {
            Some(time) if time <= since && start < high => low = start,
            // After since, or not a record, in which case reading from low still finds all after since.
            _ => high = middle,
        }
    }
    Ok(low)
}

/// When the record which is line is of, its slot, or if it has none, when it was requested, or for a period, e.g. of
/// the temperature, see climate.rs, its end.
fn line_time(line: &[u8]) -> Option<DateTime<Utc>> {
    #[derive(Deserialize)]
    struct Times {
        slot: Option<String>,
        requested: Option<String>,
        end: Option<String>,
    }

    let times = serde_json::from_slice::<Times>(line).ok()?;
    times
        .slot
        .or(times.requested)
        .or(times.end)
        .and_then(|time| DateTime::parse_from_rfc3339(&time).ok())
        .map(|time| time.with_timezone(&Utc))
}
