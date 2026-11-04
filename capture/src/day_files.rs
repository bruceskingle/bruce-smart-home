//! Records kept on the SD card, a file a day, e.g. /sd/capture/tesla/2026-10-29.jsonl, each record a line of JSON in
//! the form the app saves, written as it is taken, so that they survive the device restarting, and given to the app
//! from there, see respond. Each feature which captures keeps its own, in a directory of its own, and its latest records
//! in RAM too, for its screen, and for the app when there is no card.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::Context;
use chrono::{DateTime, Utc};
use log::warn;
use serde::Deserialize;

/// How many days of records are given when the app does not say since when, see write_since, so that a full card is not
/// sent all at once.
const DAYS_WITHOUT_SINCE: i64 = 2;

pub struct DayFiles {
    /// The directory of the files, if there is an SD card.
    dir: Option<PathBuf>,
    /// Why the records are only in RAM, if they are, e.g. there is no SD card, or the last write failed.
    problem: Option<String>,
}

impl DayFiles {
    /// dir is where the files are kept, or why there is nowhere, see Esp32Platform::sd_card.
    pub fn new(dir: Result<PathBuf, String>) -> DayFiles {
        match dir {
            Ok(dir) => DayFiles { dir: Some(dir), problem: None },
            Err(problem) => DayFiles { dir: None, problem: Some(problem) },
        }
    }

    /// Adds record, of time, as its JSON, to its day's file, if there is a card, noting the problem if that fails.
    pub fn append(&mut self, time: DateTime<Utc>, record: &str) {
        let Some(dir) = &self.dir else { return };
        match append_line(dir, time, record) {
            Ok(()) => self.problem = None,
            Err(error) => {
                warn!("Unable to save the record on the SD card, {error:#}");
                self.problem = Some(format!("{error:#}"));
            }
        }
    }

    /// Where the records are kept, for the app: sd, or ram if there is no card or it failed, see problem.
    pub fn storage(&self) -> &'static str {
        if self.dir.is_some() && self.problem.is_none() { "sd" } else { "ram" }
    }

    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    /// The directory of the files, if they can be used.
    pub fn usable(&self) -> Option<PathBuf> {
        self.dir.clone().filter(|_| self.problem.is_none())
    }

    /// Keeps a feature's latest records in RAM, oldest first, up to capacity, as their JSON, for the app if there is no
    /// card, or only the latest while the card keeps them, so that RAM is not used for nothing.
    pub fn keep(&self, ram: &mut std::collections::VecDeque<(DateTime<Utc>, Box<str>)>, capacity: usize,
        time: DateTime<Utc>, record: String) {
        let capacity = if self.usable().is_some() { 1 } else { capacity };
        while ram.len() >= capacity {
            ram.pop_front();
        }
        ram.push_back((time, record.into()));
    }
}

fn append_line(dir: &Path, time: DateTime<Utc>, record: &str) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("making {}", dir.display()))?;
    let path = dir.join(format!("{}.jsonl", time.format("%Y-%m-%d")));
    let mut line = Vec::with_capacity(record.len() + 1);
    line.extend_from_slice(record.as_bytes());
    line.push(b'\n');
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("opening {}", path.display()))?;
    file.write_all(&line).with_context(|| format!("writing {}", path.display()))?;
    // So that it is on the card, not only in a buffer, if the power fails.
    file.sync_all().with_context(|| format!("syncing {}", path.display()))?;
    Ok(())
}

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

/// The since parameter of the query string, an RFC 3339 time, e.g. ?since=2026-10-29T07:30:00Z
pub fn since(uri: &str) -> Option<DateTime<Utc>> {
    parameter(uri, "since")
        .and_then(|value| DateTime::parse_from_rfc3339(&value).ok())
        .map(|time| time.with_timezone(&Utc))
}

/// The limit parameter of the query string, the most records to give at once, e.g. ?limit=500, see write_since.
fn limit(uri: &str) -> Option<usize> {
    parameter(uri, "limit").and_then(|value| value.parse().ok())
}

fn parameter(uri: &str, name: &str) -> Option<String> {
    let (_, query) = uri.split_once('?')?;
    url::form_urlencoded::parse(query.as_bytes())
        .find(|(parameter, _)| parameter == name)
        .map(|(_, value)| value.into_owned())
}

/// How much of a response is sent at a time, see Chunks, rather than a chunk for each record and each comma between
/// them, which is slow, as each is sent on its own.
const CHUNK: usize = 1536;

/// Sending the response failed, e.g. the app went away, after which nothing more can be sent on the connection, as
/// the HTTP server may have sent only part of a chunk, so the response is given up, rather than taken for the card
/// failing, see respond.
#[derive(Debug)]
struct SendFailed(String);

impl std::fmt::Display for SendFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "sending the response, {}", self.0)
    }
}

impl std::error::Error for SendFailed {}

/// A response sent in pieces of about CHUNK bytes.
struct Chunks<W> {
    out: W,
    buffer: Vec<u8>,
}

impl<W: esp_idf_svc::io::Write> Chunks<W> {
    fn write(&mut self, bytes: &[u8]) -> anyhow::Result<()> {
        self.buffer.extend_from_slice(bytes);
        if self.buffer.len() >= CHUNK {
            self.flush()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> anyhow::Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        let result = self.out.write_all(&self.buffer);
        self.buffer.clear();
        result.map_err(|error| SendFailed(format!("{error:?}")).into())
    }
}

/// Responds to a request for the records since the time in its since parameter, up to its limit parameter, if it has
/// one, as {"device": device, "records": [...], "more": true}, more only if there are more, see write_since: from files, the feature's directory if its files can be used, see DayFiles::usable, otherwise with ram, which
/// writes, with the function it is given, the feature's records in RAM since the time it is given, which it also does if
/// the files cannot be read. Written a record at a time, as all of them would not fit in the RAM as one piece of JSON. If
/// reading them fails, the error is at the end, as recordsError, after the records before it, which the app can still
/// save. If sending fails, nothing more is sent, see SendFailed.
pub fn respond(
    request: esp_idf_svc::http::server::Request<&mut esp_idf_svc::http::server::EspHttpConnection<'_>>,
    device: &serde_json::Value,
    files: Option<PathBuf>,
    ram: impl FnOnce(Option<DateTime<Utc>>, &mut dyn FnMut(&[u8]) -> anyhow::Result<()>) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let since = since(request.uri());
    let limit = limit(request.uri());
    let response = request.into_response(200, None, &[("Content-Type", "application/json")])?;
    let mut out = Chunks { out: response, buffer: Vec::with_capacity(CHUNK + 512) };
    out.write(b"{\"device\":")?;
    out.write(&serde_json::to_vec(device)?)?;
    out.write(b",\"records\":[")?;
    let mut first = true;
    let mut write = |record: &[u8]| -> anyhow::Result<()> {
        if !first {
            out.write(b",")?;
        }
        first = false;
        out.write(record)
    };
    let mut more = false;
    let result = match files {
        // If the card cannot be read, the records in RAM too, which the app saves only if it has not already.
        Some(dir) => write_since(&dir, since, limit, &mut write).map(|(_, rest)| more = rest).or_else(|error| {
            if error.is::<SendFailed>() {
                return Err(error);
            }
            warn!("Unable to read the records on the SD card, giving those in RAM, {error:#}");
            ram(since, &mut write).and(Err(error))
        }),
        None => ram(since, &mut write),
    };
    if let Err(error) = &result
        && error.is::<SendFailed>()
    {
        warn!("Unable to send the records, {error:#}");
        return result;
    }
    out.write(b"]")?;
    if more {
        out.write(b",\"more\":true")?;
    }
    if let Err(error) = result {
        warn!("Unable to read the records, {error:#}");
        out.write(b",\"recordsError\":")?;
        out.write(&serde_json::to_vec(&format!("{error:#}"))?)?;
    }
    out.write(b"}")?;
    out.flush()
}
