//! The settings box: which sources whypkg reads. `↵` turns the source under
//! the cursor on or off and `d` puts it back as it ships (read); each change
//! is written to the settings file at once, and the browser takes it in when
//! the box closes.

use crate::config::Config;
use crate::model::Source;

pub struct Options {
    /// The sources skipped, as written.
    pub skip: Vec<Source>,
    /// The sources skipped when the box opened, to tell whether closing it
    /// changes anything.
    pub opened: Vec<Source>,
    pub cursor: usize,
}

impl Options {
    pub fn open(config: &Config) -> Options {
        Options {
            skip: config.skip.clone(),
            opened: config.skip.clone(),
            cursor: 0,
        }
    }

    pub fn changed(&self) -> bool {
        let mut a = self.skip.clone();
        let mut b = self.opened.clone();
        a.sort_by_key(|s| s.word());
        b.sort_by_key(|s| s.word());
        a != b
    }

    fn selected(&self) -> Source {
        Source::ALL[self.cursor]
    }

    pub fn toggle(&mut self) {
        let source = self.selected();
        match self.skip.iter().position(|s| *s == source) {
            Some(i) => {
                self.skip.remove(i);
            }
            None => self.skip.push(source),
        }
    }

    /// Back as it ships: read.
    pub fn default(&mut self) {
        let source = self.selected();
        self.skip.retain(|s| *s != source);
    }

    pub fn move_by(&mut self, delta: isize) {
        let last = Source::ALL.len() as isize - 1;
        self.cursor = (self.cursor as isize + delta).clamp(0, last) as usize;
    }

    /// What the file says, in the order the sources are listed.
    pub fn config(&self) -> Config {
        Config {
            skip: Source::ALL
                .into_iter()
                .filter(|s| self.skip.contains(s))
                .collect(),
        }
    }
}
