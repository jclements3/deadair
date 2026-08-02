use serde::{Deserialize, Serialize};

/// Identifier for a node in the scene graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NodeId(pub u64);

/// Identifier for a simulated entity (animal, zombie, player, prop).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EntityId(pub u64);

/// Monotonic id generator. Ids are never reused within a session.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct IdGen {
    next: u64,
}

impl IdGen {
    pub fn new() -> Self {
        Self { next: 1 }
    }

    pub fn node(&mut self) -> NodeId {
        let id = self.next;
        self.next += 1;
        NodeId(id)
    }

    pub fn entity(&mut self) -> EntityId {
        let id = self.next;
        self.next += 1;
        EntityId(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_monotonic() {
        let mut g = IdGen::new();
        let a = g.node();
        let b = g.node();
        let e = g.entity();
        assert!(a.0 < b.0 && b.0 < e.0);
    }
}
