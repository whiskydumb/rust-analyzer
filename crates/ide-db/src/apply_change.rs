//! Applies changes to the IDE state transactionally.

use std::time::{Duration, Instant};

use profile::Bytes;
use salsa::Database as _;

use crate::{ChangeWithProcMacros, RootDatabase};

impl RootDatabase {
    pub fn apply_change(&mut self, change: ChangeWithProcMacros) -> Duration {
        let _p = tracing::info_span!("RootDatabase::apply_change").entered();
        let now = Instant::now();
        self.trigger_cancellation();
        let elapsed = now.elapsed();
        tracing::trace!("apply_change {:?}", change);
        change.apply(self);
        elapsed
    }

    // Feature: Memory Usage
    //
    // Prints memory usage statistics for the internal database.
    //
    // | Editor  | Action Name |
    // |---------|-------------|
    // | VS Code | **rust-analyzer: Memory Usage**
    //
    // ![Memory Usage](https://user-images.githubusercontent.com/48062697/113065592-08559f00-91b1-11eb-8c96-64b88068ec02.gif)
    pub fn per_query_memory_usage(&mut self) -> Vec<(String, Bytes, usize)> {
        // NOTE: this takes the salsa cancellation barrier; it must not be called
        // while other database snapshots are alive on this thread.
        let db: &mut dyn salsa::Database = self;
        let info = db.memory_usage();
        let total = |it: &salsa::IngredientInfo| {
            Bytes::new(
                (it.size_of_fields()
                    + it.size_of_metadata()
                    + it.heap_size_of_fields().unwrap_or(0)) as isize,
            )
        };
        let mut acc: Vec<(String, Bytes, usize)> = Vec::new();
        for s in &info.structs {
            acc.push((format!("{} (struct)", s.debug_name()), total(s), s.count()));
        }
        for (name, q) in &info.queries {
            acc.push((name.to_string(), total(q), q.count()));
        }
        acc.sort_by_key(|it| std::cmp::Reverse(it.1));
        acc
    }
}
