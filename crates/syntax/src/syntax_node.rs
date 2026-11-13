//! This module defines Concrete Syntax Tree (CST), used by rust-analyzer.
//!
//! The CST includes comments and whitespace, provides a single node type,
//! `SyntaxNode`, and a basic traversal API (parent, children, siblings).
//!
//! The *real* implementation is in the (language-agnostic) `rowan` crate, this
//! module just wraps its API.

use std::cell::{Cell, RefCell};

use rowan::{GreenNodeBuilder, Language, NodeCache};

use crate::{Parse, SyntaxError, SyntaxKind, TextSize};

pub(crate) use rowan::{GreenNode, GreenToken, NodeOrToken};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RustLanguage {}
impl Language for RustLanguage {
    type Kind = SyntaxKind;

    fn kind_from_raw(raw: rowan::SyntaxKind) -> SyntaxKind {
        SyntaxKind::from(raw.0)
    }

    fn kind_to_raw(kind: SyntaxKind) -> rowan::SyntaxKind {
        rowan::SyntaxKind(kind.into())
    }
}

pub type SyntaxNode = rowan::SyntaxNode<RustLanguage>;
pub type SyntaxToken = rowan::SyntaxToken<RustLanguage>;
pub type SyntaxElement = rowan::SyntaxElement<RustLanguage>;
pub type SyntaxNodeChildren = rowan::SyntaxNodeChildren<RustLanguage>;
pub type SyntaxElementChildren = rowan::SyntaxElementChildren<RustLanguage>;
pub type PreorderWithTokens = rowan::api::PreorderWithTokens<RustLanguage>;

/// A node cache shared between parses on the calling thread.
///
/// Identical green nodes and tokens deduplicate across files (and macro
/// expansions), not just within a single parse. The cache is periodically
/// reset so that it cannot pin green data of evicted trees forever.
///
/// Note this intentionally hands out aliased `&mut` references, which relies
/// on builders never being used re-entrantly on the same thread; green
/// building is a pure bottom-up process that cannot call back into parsing.
fn shared_node_cache() -> &'static mut NodeCache {
    /// Number of tree builds between cache resets.
    const RESET_EVERY: usize = 64;

    thread_local! {
        static CACHE: RefCell<Option<*mut NodeCache>> = const { RefCell::new(None) };
        static USES: Cell<usize> = const { Cell::new(0) };
    }

    let uses = USES.replace(USES.get() + 1);
    CACHE.with(|cache| {
        let cache = &mut *cache.borrow_mut();
        let cache = cache.get_or_insert_with(|| Box::leak(Box::new(NodeCache::default())));
        // SAFETY: this intentionally hands out aliased `&mut` references to the
        // shared cache. It is sound because a `NodeCache` is only mutated
        // through plain `HashMap` operations inside the green builder, and two
        // builders can never be used re-entrantly on the same thread: green
        // building is a pure bottom-up process that cannot call back into
        // parsing or expansion.
        unsafe {
            if uses >= RESET_EVERY {
                USES.set(0);
                **cache = NodeCache::default();
            }
            &mut **cache
        }
    })
}

pub struct SyntaxTreeBuilder {
    errors: Vec<SyntaxError>,
    inner: GreenNodeBuilder<'static>,
}

impl Default for SyntaxTreeBuilder {
    fn default() -> Self {
        Self { errors: Vec::new(), inner: GreenNodeBuilder::with_cache(shared_node_cache()) }
    }
}

impl SyntaxTreeBuilder {
    pub(crate) fn finish_raw(self) -> (GreenNode, Vec<SyntaxError>) {
        let green = self.inner.finish();
        // The shared node cache deduplicates nodes with up to 3 children, so
        // a whole small tree can end up shared between files. `SyntaxNode`
        // identity is green-pointer based though, and e.g. `Semantics`'s
        // root-to-file map requires each parse to own a unique root.
        let green = if green.children().len() <= 3 {
            GreenNode::new(green.kind(), green.children().map(|it| it.to_owned()))
        } else {
            green
        };
        (green, self.errors)
    }

    pub fn finish(self) -> Parse<SyntaxNode> {
        let (green, errors) = self.finish_raw();
        // Disable block validation, see https://github.com/rust-lang/rust-analyzer/pull/10357
        #[allow(clippy::overly_complex_bool_expr)]
        if cfg!(debug_assertions) && false {
            let node = SyntaxNode::new_root(green.clone());
            crate::validation::validate_block_structure(&node);
        }
        Parse::new(green, errors)
    }

    pub fn token(&mut self, kind: SyntaxKind, text: &str) {
        let kind = RustLanguage::kind_to_raw(kind);
        self.inner.token(kind, text);
    }

    pub fn start_node(&mut self, kind: SyntaxKind) {
        let kind = RustLanguage::kind_to_raw(kind);
        self.inner.start_node(kind);
    }

    pub fn finish_node(&mut self) {
        self.inner.finish_node();
    }

    pub fn error(&mut self, error: String, text_pos: TextSize) {
        self.errors.push(SyntaxError::new_at_offset(error, text_pos));
    }
}
