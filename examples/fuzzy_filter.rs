// SPDX-License-Identifier: MPL-2.0

//! Runyte's picker ranking behind an `fzf --filter` shaped command line.
//!
//! `fzf --filter=QUERY` reads candidates on standard input and writes the
//! matches to standard output, best first. This does the same through
//! `FuzzyMatcher`, so a benchmark can hand both programs the same bytes and
//! compare what comes back as well as what it cost. It exists for
//! `benchmarks/fuzzy.py`; nothing in the editor uses it.
//!
//! ```sh
//! cargo build --release --example fuzzy_filter
//! target/release/examples/fuzzy_filter --query src/app < corpus.txt
//! ```
//!
//! Options:
//!
//! * `--query Q` — the query. Required; an empty one matches everything.
//! * `--limit N` — print only the first N matches. All of them by default.
//! * `--lines` — rank as lines of text rather than as paths.
//! * `--repeat N` — rank N times, printing the result of the last. Reading the
//!   corpus and writing the answer happen once either way, so the reported
//!   time is ranking alone.
//! * `--time` — write `rank_us <median> <query> <candidates>` to standard
//!   error, the median of the `--repeat` passes in microseconds.

use std::{
    hint::black_box,
    io::{self, Read, Write},
    process::ExitCode,
    time::Instant,
};

use runyte::file_picker::{FuzzyCandidate, FuzzyMatcher};

struct Options {
    query: String,
    limit: Option<usize>,
    kind: FuzzyCandidate,
    repeat: usize,
    time: bool,
}

fn parse_arguments(arguments: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut query = None;
    let mut limit = None;
    let mut kind = FuzzyCandidate::Path;
    let mut repeat = 1;
    let mut time = false;
    let mut arguments = arguments.peekable();
    while let Some(argument) = arguments.next() {
        // `--query=x` and `--query x` are both what a caller expects to work,
        // and fzf accepts the joined spelling for `--filter`.
        let (name, joined) = match argument.split_once('=') {
            Some((name, value)) => (name.to_owned(), Some(value.to_owned())),
            None => (argument, None),
        };
        let mut value = || -> Result<String, String> {
            joined
                .clone()
                .or_else(|| arguments.next())
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match name.as_str() {
            "--query" | "--filter" | "-f" => query = Some(value()?),
            "--limit" => {
                limit = Some(
                    value()?
                        .parse()
                        .map_err(|_| "--limit needs a number".to_owned())?,
                );
            }
            "--repeat" => {
                repeat = value()?
                    .parse()
                    .map_err(|_| "--repeat needs a number".to_owned())?;
                if repeat == 0 {
                    return Err("--repeat needs at least one pass".to_owned());
                }
            }
            "--lines" => kind = FuzzyCandidate::Line,
            "--time" => time = true,
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(Options {
        query: query.ok_or_else(|| "--query is required".to_owned())?,
        limit,
        kind,
        repeat,
        time,
    })
}

/// One ranking pass, ordered the way the file picker orders its own matches:
/// score first, then the shorter candidate, then the candidate itself, so that
/// equal scores come out in a stable order rather than in corpus order. An
/// empty query scores everything the same, and the picker orders that case by
/// path alone rather than by a length that means nothing here.
fn rank<'a>(query: &str, kind: FuzzyCandidate, candidates: &[&'a str]) -> Vec<(i64, &'a str)> {
    let mut matcher = FuzzyMatcher::for_candidate(query, kind);
    let mut matches = candidates
        .iter()
        .filter_map(|candidate| {
            matcher
                .score(candidate)
                .map(|(score, _)| (score, *candidate))
        })
        .collect::<Vec<_>>();
    if query.is_empty() {
        matches.sort_by_key(|(_, candidate)| *candidate);
        return matches;
    }
    matches.sort_by(|(left_score, left), (right_score, right)| {
        right_score
            .cmp(left_score)
            .then_with(|| left.chars().count().cmp(&right.chars().count()))
            .then_with(|| left.cmp(right))
    });
    matches
}

fn main() -> ExitCode {
    let options = match parse_arguments(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("fuzzy_filter: {message}");
            return ExitCode::FAILURE;
        }
    };

    let mut corpus = String::new();
    if let Err(error) = io::stdin().read_to_string(&mut corpus) {
        eprintln!("fuzzy_filter: reading candidates: {error}");
        return ExitCode::FAILURE;
    }
    let candidates = corpus.lines().collect::<Vec<_>>();

    let mut samples = Vec::with_capacity(options.repeat);
    let mut matches = Vec::new();
    for pass in 0..options.repeat {
        let start = Instant::now();
        let ranked = rank(&options.query, options.kind, &candidates);
        samples.push(start.elapsed());
        // Keeping the last pass rather than the first means the optimizer
        // cannot drop the earlier ones as unused work.
        if pass + 1 == options.repeat {
            matches = ranked;
        } else {
            black_box(&ranked);
        }
    }
    samples.sort_unstable();
    let median = samples[samples.len() / 2];

    let mut out = io::BufWriter::new(io::stdout().lock());
    for (_, candidate) in matches.iter().take(options.limit.unwrap_or(usize::MAX)) {
        if writeln!(out, "{candidate}").is_err() {
            // A closed pipe is how `head` ends a filter, not a failure.
            return ExitCode::SUCCESS;
        }
    }
    if out.flush().is_err() {
        return ExitCode::SUCCESS;
    }

    if options.time {
        eprintln!(
            "rank_us {:.3} {} {}",
            median.as_secs_f64() * 1_000_000.0,
            options.query,
            candidates.len()
        );
    }
    ExitCode::SUCCESS
}
