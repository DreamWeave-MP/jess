//! The routing graph as the control side keeps it, and the order the engine mixes buses in.
//!
//! The engine never decides an order: it walks the one it was last sent. Every change to the
//! routing is checked here first, a new order is worked out, and the change and its order go to
//! the engine in one command, so it never mixes a graph that is half changed. A change that
//! would route a bus into itself, however far round, is refused before anything is sent.

use std::collections::BTreeSet;

use crate::audio::{AudioError, BusKey};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Free,
    Live,
    /// Sent to the engine to be removed; its slot is the engine's until it says the bus ended.
    Removing,
}

#[derive(Clone, Debug)]
struct Node {
    generation: u32,
    state: State,
    parent: Option<u32>,
}

impl Node {
    /// The buses this one feeds.
    fn outputs(&self) -> impl Iterator<Item = u32> {
        self.parent.into_iter()
    }
}

/// The buses and how they connect. Slot 0 is the master bus.
#[derive(Clone, Debug)]
pub struct Graph {
    nodes: Vec<Node>,
    /// Free slots, the next one at the end.
    free: Vec<u32>,
}

impl Graph {
    /// A graph of `capacity` bus slots with the master bus live in slot 0.
    pub fn new(capacity: usize) -> Graph {
        let mut nodes = vec![
            Node {
                generation: 0,
                state: State::Free,
                parent: None,
            };
            capacity
        ];
        nodes[0].state = State::Live;
        let free = (1..u32::try_from(capacity).unwrap_or(u32::MAX)).rev().collect();
        Graph { nodes, free }
    }

    pub fn capacity(&self) -> usize {
        self.nodes.len()
    }

    /// True for a bus that is live: made, and not being removed.
    pub fn is_live(&self, bus: BusKey) -> bool {
        self.nodes
            .get(bus.slot as usize)
            .is_some_and(|node| node.state == State::Live && node.generation == bus.generation)
    }

    pub fn check_live(&self, bus: BusKey) -> Result<(), AudioError> {
        if self.is_live(bus) {
            return Ok(());
        }
        Err(AudioError::StaleBus)
    }

    /// Takes a free slot for a bus under `parent`, or `None` when every slot is taken.
    pub fn add(&mut self, parent: BusKey) -> Option<BusKey> {
        let slot = self.free.pop()?;
        let node = &mut self.nodes[slot as usize];
        node.state = State::Live;
        node.parent = Some(parent.slot);
        Some(BusKey {
            slot,
            generation: node.generation,
        })
    }

    /// Gives back a slot `add` took that never reached the engine.
    pub fn unadd(&mut self, bus: BusKey) {
        let node = &mut self.nodes[bus.slot as usize];
        node.state = State::Free;
        node.parent = None;
        self.free.push(bus.slot);
    }

    /// The live buses whose parent is `bus`, counting ones being removed: they still sum into it
    /// until the engine is done with them.
    pub fn children(&self, bus: BusKey) -> usize {
        self.nodes
            .iter()
            .filter(|node| node.state != State::Free && node.parent == Some(bus.slot))
            .count()
    }

    pub fn mark_removing(&mut self, bus: BusKey) {
        self.nodes[bus.slot as usize].state = State::Removing;
    }

    /// The engine is done with `bus`: its slot is free, and the next bus in it is a new one.
    pub fn ended(&mut self, bus: BusKey) {
        let node = &mut self.nodes[bus.slot as usize];
        if node.generation != bus.generation || node.state == State::Free {
            return;
        }
        node.generation = node.generation.wrapping_add(1);
        node.state = State::Free;
        node.parent = None;
        self.free.push(bus.slot);
    }

    /// Reroutes `bus` into `parent`, or leaves the graph as it was and says why not.
    pub fn route(&mut self, bus: BusKey, parent: BusKey) -> Result<Box<[u32]>, AudioError> {
        let node = &mut self.nodes[bus.slot as usize];
        let old = node.parent.replace(parent.slot);
        match self.order() {
            Ok(order) => Ok(order),
            Err(error) => {
                self.nodes[bus.slot as usize].parent = old;
                Err(error)
            }
        }
    }

    /// Every bus not yet free in an order where each comes before everything it feeds, the
    /// master last. Among buses that could go next, the lowest slot goes first, so the same graph
    /// always mixes in the same order.
    pub fn order(&self) -> Result<Box<[u32]>, AudioError> {
        let present = |slot: usize| self.nodes[slot].state != State::Free;
        let mut waiting = vec![0usize; self.nodes.len()];
        for (slot, node) in self.nodes.iter().enumerate() {
            if !present(slot) {
                continue;
            }
            for target in node.outputs() {
                waiting[target as usize] += 1;
            }
        }
        let mut ready: BTreeSet<u32> = (0..self.nodes.len())
            .filter(|slot| present(*slot) && waiting[*slot] == 0)
            .filter_map(|slot| u32::try_from(slot).ok())
            .collect();
        let mut order = Vec::with_capacity(self.nodes.len());
        while let Some(slot) = ready.pop_first() {
            order.push(slot);
            for target in self.nodes[slot as usize].outputs() {
                waiting[target as usize] -= 1;
                if waiting[target as usize] == 0 {
                    ready.insert(target);
                }
            }
        }
        let count = (0..self.nodes.len()).filter(|slot| present(*slot)).count();
        if order.len() < count {
            return Err(AudioError::Cycle);
        }
        Ok(order.into_boxed_slice())
    }
}
