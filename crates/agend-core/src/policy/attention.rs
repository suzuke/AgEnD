//! Order of needs-you items (D36). The operator's attention is the scarce
//! resource, so the item that lets the most work continue comes first; among
//! equals the one waiting longest; ties break by id so the order is stable.
//!
//! Must NOT: read the clock or count blocked work itself; the daemon passes
//! both in.

use alloc::string::String;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttentionItem {
    pub id: String,
    /// Tasks and agents that can continue once this item is resolved.
    pub unblocks: u32,
    /// When the item started waiting; smaller is older.
    pub waiting_since_unix_ms: u64,
}

/// Sort in place: most unblocked first, then oldest, then by id.
pub fn order(items: &mut [AttentionItem]) {
    items.sort_by(|a, b| {
        b.unblocks
            .cmp(&a.unblocks)
            .then(a.waiting_since_unix_ms.cmp(&b.waiting_since_unix_ms))
            .then_with(|| a.id.cmp(&b.id))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    fn item(id: &str, unblocks: u32, waiting_since_unix_ms: u64) -> AttentionItem {
        AttentionItem {
            id: id.into(),
            unblocks,
            waiting_since_unix_ms,
        }
    }

    fn ids(items: &[AttentionItem]) -> Vec<&str> {
        items.iter().map(|item| item.id.as_str()).collect()
    }

    #[test]
    fn most_unblocking_item_comes_first_even_if_newer() {
        let mut items = vec![
            item("old", 1, 100),
            item("wide", 4, 900),
            item("mid", 2, 500),
        ];
        order(&mut items);
        assert_eq!(ids(&items), ["wide", "mid", "old"]);
    }

    #[test]
    fn equal_impact_puts_the_longest_waiting_first() {
        let mut items = vec![item("b", 2, 300), item("a", 2, 100), item("c", 2, 200)];
        order(&mut items);
        assert_eq!(ids(&items), ["a", "c", "b"]);
    }

    #[test]
    fn full_ties_break_by_id_regardless_of_input_order() {
        let mut forward = vec![item("x", 1, 50), item("y", 1, 50), item("w", 1, 50)];
        let mut backward: Vec<_> = forward.iter().rev().cloned().collect();
        order(&mut forward);
        order(&mut backward);
        assert_eq!(ids(&forward), ["w", "x", "y"]);
        assert_eq!(forward, backward);
    }
}
