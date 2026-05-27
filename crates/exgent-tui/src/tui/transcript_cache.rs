use std::collections::{HashMap, VecDeque};

use ratatui::text::Line;

const DEFAULT_CAPACITY: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    index: usize,
    width: usize,
    revision: u64,
}

#[derive(Debug, Clone)]
pub(super) struct TranscriptCache {
    capacity: usize,
    entries: HashMap<Key, Vec<Line<'static>>>,
    insertion_order: VecDeque<Key>,
}

impl Default for TranscriptCache {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }
}

impl TranscriptCache {
    pub(super) fn with_capacity(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            entries: HashMap::with_capacity(capacity.max(1)),
            insertion_order: VecDeque::with_capacity(capacity.max(1)),
        }
    }

    pub(super) fn get(
        &self,
        index: usize,
        width: usize,
        revision: u64,
    ) -> Option<&[Line<'static>]> {
        self.entries
            .get(&Key {
                index,
                width,
                revision,
            })
            .map(Vec::as_slice)
    }

    pub(super) fn insert(
        &mut self,
        index: usize,
        width: usize,
        revision: u64,
        lines: Vec<Line<'static>>,
    ) {
        let key = Key {
            index,
            width,
            revision,
        };
        if self.entries.insert(key, lines).is_some() {
            return;
        }
        if self.entries.len() > self.capacity {
            if let Some(oldest) = self.insertion_order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
        self.insertion_order.push_back(key);
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
        self.insertion_order.clear();
    }
}
