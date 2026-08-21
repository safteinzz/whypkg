//! The manual/auto/local filter and the fuzzy match that narrows the list.

use super::*;

/// Which packages the list shows, cycled with Tab. Buckets are disjoint:
/// manual/auto are *system* packages only, flatpak is its own group (flatpak
/// apps are technically "manual", but as a bucket they're more useful on their
/// own). The fuzzy query composes on top.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum FilterMode {
    All,
    Manual,
    Auto,
    Flatpak,
}

impl FilterMode {
    pub(crate) fn matches(self, world: &World, name: &str) -> bool {
        let is_flatpak = world
            .packages
            .get(name)
            .map(|p| p.source == crate::model::Source::Flatpak)
            .unwrap_or(false);
        match self {
            FilterMode::All => true,
            FilterMode::Manual => world.is_manual(name) && !is_flatpak,
            FilterMode::Auto => !world.is_manual(name) && !is_flatpak,
            FilterMode::Flatpak => is_flatpak,
        }
    }
    pub(crate) fn next(self) -> Self {
        match self {
            FilterMode::All => FilterMode::Manual,
            FilterMode::Manual => FilterMode::Auto,
            FilterMode::Auto => FilterMode::Flatpak,
            FilterMode::Flatpak => FilterMode::All,
        }
    }
    pub(crate) fn label(self) -> &'static str {
        match self {
            FilterMode::All => "all",
            FilterMode::Manual => "manual [M]",
            FilterMode::Auto => "auto [A]",
            FilterMode::Flatpak => "flatpak [F]",
        }
    }
}

impl App {
    /// The current base list, filtered by manual/auto and ranked by the fuzzy
    /// query. An empty query keeps the natural (sorted) order.
    pub(crate) fn filtered(&mut self) -> Vec<String> {
        let (query, base) = {
            let base: Vec<String> = self
                .base_list()
                .iter()
                .filter(|n| self.filter.matches(&self.world, n))
                .cloned()
                .collect();
            (self.stack.last().unwrap().query.clone(), base)
        };
        if query.is_empty() {
            return base;
        }
        // Match against "name + description", not just the name, so a flatpak
        // app is findable by its friendly name (search "element" → im.riot.Riot)
        // and any package by words in its synopsis.
        let pattern = Pattern::parse(&query, CaseMatching::Ignore, Normalization::Smart);
        let world = &self.world;
        let matcher = &mut self.matcher;
        let mut buf = Vec::new();
        let mut scored: Vec<(u32, String)> = base
            .into_iter()
            .filter_map(|name| {
                let haystack = match world.packages.get(&name) {
                    Some(p) => {
                        let mut h = name.clone();
                        if !p.description.is_empty() {
                            h.push(' ');
                            h.push_str(&p.description);
                        }
                        if let Some(d) = &p.details {
                            h.push(' ');
                            h.push_str(d);
                        }
                        h
                    }
                    None => name.clone(),
                };
                let utf32 = Utf32Str::new(&haystack, &mut buf);
                pattern.score(utf32, matcher).map(|score| (score, name))
            })
            .collect();
        // Highest score first; ties keep the pool's (sorted) order via stable sort.
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        scored.into_iter().map(|(_, name)| name).collect()
    }
}
