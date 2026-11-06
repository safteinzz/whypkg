pub mod browse;
pub mod leaves;
pub mod pending;
pub mod selfcmd;
pub mod why;

use crate::load::{Progress, Sources, read_world};
use crate::model::{Source, World};

/// Parses a source's word for `--only`, so clap lists the words as its
/// possible values.
pub fn source_parser() -> impl clap::builder::TypedValueParser<Value = Source> {
    use clap::builder::TypedValueParser;
    clap::builder::PossibleValuesParser::new(Source::ALL.map(Source::word))
        .map(|word: String| Source::from_word(&word).unwrap_or(Source::System))
}

/// The sources a run reads: the ones `--only` names, or else every source but
/// the ones the settings file skips. A broken settings file stops the run.
pub fn sources(only: &[Source]) -> Sources {
    if !only.is_empty() {
        return Sources::only(only);
    }
    match crate::config::load() {
        Ok(config) => Sources::all_but(&config.skip),
        Err(e) => {
            eprintln!("whypkg: {e}");
            std::process::exit(1);
        }
    }
}

/// Build the whole [`World`] from `sources` for a report. On failure we print
/// a clear message and exit.
pub fn load_world(sources: &Sources) -> World {
    match read_world(sources, &Progress::default()) {
        Ok(world) => world,
        Err(e) => {
            eprintln!("whypkg: {e}");
            std::process::exit(1);
        }
    }
}
