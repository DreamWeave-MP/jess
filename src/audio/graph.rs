//! The graph as the control side keeps it, and the compiler that turns it into a plan.
//!
//! A [`Definition`] is what the application asked for: nodes, their ports, the connections
//! between them, and insert chains. Only the control side has one. Every change is made to a
//! clone, checked, and compiled ([`Definition::compile`]) into a [`Plan`] before anything is sent,
//! so a change that would loop, name a port that isn't there, or overrun a budget is refused
//! with the graph as it was. The engine never sees a definition, and never decides an order.

use std::collections::BTreeSet;

use crate::audio::delays::{Delays, Edge, INPUT, Rings};
use crate::audio::mixer::{NodeKey, Tap};
use crate::audio::plan::{Clear, Feedback, Place, Plan, Pull, Push, Span, Step, Target};
use crate::audio::processor::{ParamInfo, PortKind, Ports, Processor};
use crate::audio::{AudioError, QUANTUM};

/// The longest a feedback connection delays, in frames: about 21 seconds at 48 kHz.
pub const MAX_FEEDBACK: u64 = 1 << 20;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Free,
    Live,
    /// Removed; its slot is the engine's until it says the node ended.
    Removing,
}

/// What the graph knows of a node's processor: enough to check and compile connections without
/// the processor, which the engine has.
#[derive(Clone, Copy, Debug)]
pub struct Shape {
    pub kind: &'static str,
    pub ports: &'static Ports,
    pub params: &'static [ParamInfo],
    pub latency: u64,
    pub tail: u64,
    pub mixer: bool,
}

impl Shape {
    pub fn of(processor: &dyn Processor) -> Shape {
        Shape {
            kind: processor.kind(),
            ports: processor.ports(),
            params: processor.params(),
            latency: processor.latency(),
            tail: processor.tail().frames(),
            mixer: processor.mixer(),
        }
    }
}

#[derive(Clone, Debug)]
struct Node {
    generation: u32,
    state: State,
    shape: Shape,
    channels: usize,
    /// The node whose insert chain it is in.
    host: Option<u32>,
    /// Its insert chain, in order.
    inserts: Vec<u32>,
    /// The node its route connection reaches: a bus's parent.
    route: Option<u32>,
    /// Removed with a tail: it runs, unfed, until the engine says it has rung out.
    ringing: bool,
}

/// Which input of a node a connection reaches.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Input {
    /// An input port, by index among the processor's inputs.
    Port(u8),
    /// A parameter, by index: modulation.
    Param(u16),
}

/// A connection.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Link {
    from: u32,
    output: u8,
    to: u32,
    input: Input,
    level: f32,
    tap: Tap,
    /// Frames of delay on a connection that may close a cycle, or 0.
    feedback: u64,
}

/// How a connection is made: its level, its tap, and a feedback delay.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Connection {
    /// Linear level.
    pub level: f32,
    /// Where it taps its source.
    pub tap: Tap,
    /// Frames of delay, at least a quantum, for a connection that may close a cycle; `None` for
    /// one that may not.
    pub feedback: Option<u64>,
}

impl Default for Connection {
    fn default() -> Self {
        Connection {
            level: 1.0,
            tap: Tap::PostFader,
            feedback: None,
        }
    }
}

/// See the module docs. Slot 0 is the master bus.
#[derive(Clone, Debug)]
pub struct Definition {
    nodes: Vec<Node>,
    links: Vec<Link>,
    /// Free slots, the next one at the end.
    free: Vec<u32>,
    /// The connections it has room for.
    room: usize,
    channels: usize,
}

/// When a node's signal is where, in frames behind what voices add straight into the graph: the
/// latest of what reaches it, after its insert chain, and after its own processing.
#[derive(Clone, Copy, Default)]
struct Timing {
    arrival: u64,
    pre: u64,
    post: u64,
}

const FREE: Node = Node {
    generation: 0,
    state: State::Free,
    shape: Shape {
        kind: "",
        ports: &Ports::EFFECT,
        params: &[],
        latency: 0,
        tail: 0,
        mixer: false,
    },
    channels: 0,
    host: None,
    inserts: Vec::new(),
    route: None,
    ringing: false,
};

impl Definition {
    /// A graph of `capacity` node slots and room for `connections`, in a context of `channels`,
    /// with the master bus, `master`, live in slot 0.
    pub fn new(capacity: usize, connections: usize, channels: usize, master: Shape) -> Definition {
        let mut nodes = vec![FREE; capacity];
        nodes[0].state = State::Live;
        nodes[0].shape = master;
        nodes[0].channels = channels;
        let free = (1..u32::try_from(capacity).unwrap_or(u32::MAX)).rev().collect();
        Definition {
            nodes,
            links: Vec::new(),
            free,
            room: connections,
            channels,
        }
    }

    fn node(&self, key: NodeKey) -> Option<&Node> {
        self.nodes
            .get(key.slot as usize)
            .filter(|node| node.state == State::Live && node.generation == key.generation)
    }

    /// True for a node that is live: made, and not removed.
    pub fn is_live(&self, key: NodeKey) -> bool {
        self.node(key).is_some()
    }

    /// # Errors
    ///
    /// [`AudioError::StaleNode`] for a node that isn't live.
    pub fn check_live(&self, key: NodeKey) -> Result<(), AudioError> {
        if self.is_live(key) {
            return Ok(());
        }
        Err(AudioError::StaleNode)
    }

    /// The key of the node in `slot` now.
    pub fn key(&self, slot: u32) -> NodeKey {
        NodeKey {
            slot,
            generation: self.nodes[slot as usize].generation,
        }
    }

    /// What `key` is, if it is live.
    pub fn shape(&self, key: NodeKey) -> Option<&Shape> {
        self.node(key).map(|node| &node.shape)
    }

    /// The channels of node `slot`, live or not.
    pub fn channels(&self, slot: u32) -> usize {
        self.nodes[slot as usize].channels
    }

    /// True for a node voices can play into: live, with an audio input, not in an insert chain.
    pub fn takes_voices(&self, key: NodeKey) -> bool {
        self.node(key)
            .is_some_and(|node| node.host.is_none() && node.shape.ports.audio().next().is_some())
    }

    /// Takes a free slot for a node of `shape` with `channels`.
    ///
    /// # Errors
    ///
    /// [`AudioError::NodesExhausted`] when every slot is taken, and [`AudioError::Channels`] for
    /// more channels than the context has.
    pub fn add(&mut self, shape: Shape, channels: Option<usize>) -> Result<NodeKey, AudioError> {
        let channels = channels.unwrap_or(self.channels);
        if channels == 0 || channels > self.channels {
            return Err(AudioError::Channels(channels));
        }
        let Some(slot) = self.free.pop() else {
            return Err(AudioError::NodesExhausted(self.nodes.len()));
        };
        let node = &mut self.nodes[slot as usize];
        node.state = State::Live;
        node.shape = shape;
        node.channels = channels;
        Ok(NodeKey {
            slot,
            generation: node.generation,
        })
    }

    /// Output `name` of `key`.
    fn output(&self, key: NodeKey, name: &str) -> Result<u8, AudioError> {
        let node = self.node(key).ok_or(AudioError::StaleNode)?;
        let outputs = node.shape.ports.outputs;
        let index = outputs
            .iter()
            .position(|known| *known == name)
            .ok_or_else(|| AudioError::UnknownPort {
                kind: node.shape.kind,
                name: name.to_owned(),
                known: outputs.join(", "),
            })?;
        Ok(u8::try_from(index).unwrap_or(u8::MAX))
    }

    /// Input `name` of `key`: an input port, or a parameter to modulate.
    fn input(&self, key: NodeKey, name: &str) -> Result<Input, AudioError> {
        let node = self.node(key).ok_or(AudioError::StaleNode)?;
        let shape = &node.shape;
        if let Some(index) = shape.ports.inputs.iter().position(|port| port.name == name) {
            return Ok(Input::Port(u8::try_from(index).unwrap_or(u8::MAX)));
        }
        if let Some(index) = shape.params.iter().position(|param| param.name == name) {
            return Ok(Input::Param(u16::try_from(index).unwrap_or(u16::MAX)));
        }
        let known: Vec<&str> = shape
            .ports
            .inputs
            .iter()
            .map(|port| port.name)
            .chain(shape.params.iter().map(|param| param.name))
            .collect();
        Err(AudioError::UnknownPort {
            kind: shape.kind,
            name: name.to_owned(),
            known: known.join(", "),
        })
    }

    fn find(&self, from: u32, output: u8, to: u32, input: Input) -> Option<usize> {
        self.links
            .iter()
            .position(|link| link.from == from && link.output == output && link.to == to && link.input == input)
    }

    /// Connects output `output` of `from` to input `input` of `to`, or changes the connection
    /// already there.
    ///
    /// # Errors
    ///
    /// [`AudioError::StaleNode`], [`AudioError::UnknownPort`], [`AudioError::Layout`] for channel
    /// counts that don't convert, [`AudioError::Inserted`] for an inserted processor's output or
    /// main input, [`AudioError::SidechainTaken`], [`AudioError::Parameter`] for a level that
    /// isn't finite or a feedback delay out of range, [`AudioError::Unsupported`] for a tap before
    /// the processing on an output other than the first, or feedback into anything but an audio
    /// input, and [`AudioError::ConnectionsExhausted`]. A loop is refused when the graph compiles.
    pub fn connect(
        &mut self,
        from: NodeKey,
        output: &str,
        to: NodeKey,
        input: &str,
        connection: Connection,
    ) -> Result<(), AudioError> {
        let output = self.output(from, output)?;
        let target = self.input(to, input)?;
        if !connection.level.is_finite() {
            return Err(AudioError::Parameter {
                name: "connection level",
                value: f64::from(connection.level),
                expected: "a finite number",
            });
        }
        let source = &self.nodes[from.slot as usize];
        let sink = &self.nodes[to.slot as usize];
        if source.host.is_some() {
            return Err(AudioError::Inserted);
        }
        if connection.tap == Tap::PreFader && output != 0 {
            return Err(AudioError::Unsupported(
                "a tap before a node's processing is of its first output",
            ));
        }
        let feedback = connection.feedback.unwrap_or(0);
        if connection.feedback.is_some() && !(QUANTUM as u64..=MAX_FEEDBACK).contains(&feedback) {
            #[allow(clippy::cast_precision_loss)] // Frames in a message.
            return Err(AudioError::Parameter {
                name: "feedback",
                value: feedback as f64,
                expected: "a delay of 128 frames to 2^20",
            });
        }
        if let Input::Port(port) = target {
            let kind = sink.shape.ports.inputs[port as usize].kind;
            if kind == PortKind::Audio && sink.host.is_some() {
                return Err(AudioError::Inserted);
            }
            if kind == PortKind::Sidechain
                && self.links.iter().any(|link| {
                    link.to == to.slot && link.input == target && (link.from, link.output) != (from.slot, output)
                })
            {
                return Err(AudioError::SidechainTaken);
            }
            if feedback > 0 && kind != PortKind::Audio {
                return Err(AudioError::Unsupported("feedback reaches an audio input"));
            }
            let (source, output) = (source.channels, sink.channels);
            if source != output && source != 1 && output != 1 {
                return Err(AudioError::Layout { source, output });
            }
        } else if feedback > 0 {
            return Err(AudioError::Unsupported("feedback reaches an audio input"));
        }
        let link = Link {
            from: from.slot,
            output,
            to: to.slot,
            input: target,
            level: connection.level,
            tap: connection.tap,
            feedback,
        };
        if let Some(index) = self.find(from.slot, output, to.slot, target) {
            self.links[index] = link;
            return Ok(());
        }
        if self.links.len() == self.room {
            return Err(AudioError::ConnectionsExhausted(self.room));
        }
        self.links.push(link);
        Ok(())
    }

    /// Takes away the connection from output `output` of `from` to input `input` of `to`; true if
    /// there was one.
    ///
    /// # Errors
    ///
    /// [`AudioError::StaleNode`] or [`AudioError::UnknownPort`].
    pub fn disconnect(&mut self, from: NodeKey, output: &str, to: NodeKey, input: &str) -> Result<bool, AudioError> {
        let output = self.output(from, output)?;
        let input = self.input(to, input)?;
        let Some(index) = self.find(from.slot, output, to.slot, input) else {
            return Ok(false);
        };
        self.links.swap_remove(index);
        if self.nodes[from.slot as usize].route == Some(to.slot) && output == 0 && input == Input::Port(0) {
            self.nodes[from.slot as usize].route = None;
        }
        Ok(true)
    }

    /// Takes away whatever reaches `to`'s sidechain.
    pub fn unlisten(&mut self, to: NodeKey) {
        self.links.retain(|link| {
            link.to != to.slot
                || !matches!(link.input, Input::Port(port) if self.nodes[to.slot as usize].shape.ports.inputs[port as usize].kind == PortKind::Sidechain)
        });
    }

    /// Routes `node` into `parent`: replaces its route connection, at unity after its processing,
    /// keeping every other connection it has.
    ///
    /// # Errors
    ///
    /// [`AudioError::Master`] for the master, and as [`Definition::connect`].
    pub fn route(&mut self, node: NodeKey, parent: NodeKey) -> Result<(), AudioError> {
        if node == NodeKey::MASTER {
            return Err(AudioError::Master);
        }
        self.check_live(node)?;
        self.check_live(parent)?;
        if let Some(old) = self.nodes[node.slot as usize].route.take() {
            let old = self.key(old);
            self.disconnect(node, "out", old, "in")?;
        }
        let output = self.nodes[node.slot as usize].shape.ports.outputs[0];
        let input = self.nodes[parent.slot as usize]
            .shape
            .ports
            .audio()
            .next()
            .map_or("in", |port| port.name);
        self.connect(node, output, parent, input, Connection::default())?;
        self.nodes[node.slot as usize].route = Some(parent.slot);
        Ok(())
    }

    /// The connections reaching `node`'s audio inputs.
    pub fn feeds(&self, node: NodeKey) -> usize {
        let shape = &self.nodes[node.slot as usize].shape;
        self.links
            .iter()
            .filter(|link| {
                link.to == node.slot
                    && matches!(link.input, Input::Port(port) if shape.ports.inputs[port as usize].kind == PortKind::Audio)
            })
            .count()
    }

    /// Puts `node` in `host`'s insert chain at `index`, or last.
    ///
    /// # Errors
    ///
    /// [`AudioError::StaleNode`], and [`AudioError::NotInsertable`] for a node with connections
    /// to its audio input or from its output, of another shape or channel count than a chain
    /// holds, already in a chain, or a host that is itself in one.
    pub fn insert(&mut self, host: NodeKey, node: NodeKey, index: Option<usize>) -> Result<(), AudioError> {
        self.check_live(host)?;
        self.check_live(node)?;
        let candidate = &self.nodes[node.slot as usize];
        let wired = self
            .links
            .iter()
            .any(|link| link.from == node.slot || (link.to == node.slot && link.input == Input::Port(0)));
        if host == node
            || candidate.host.is_some()
            || !candidate.inserts.is_empty()
            || candidate.shape.mixer
            || !candidate.shape.ports.insertable()
            || candidate.channels != self.nodes[host.slot as usize].channels
            || self.nodes[host.slot as usize].host.is_some()
            || wired
        {
            return Err(AudioError::NotInsertable);
        }
        let chain = &mut self.nodes[host.slot as usize].inserts;
        let index = index.unwrap_or(chain.len()).min(chain.len());
        chain.insert(index, node.slot);
        self.nodes[node.slot as usize].host = Some(host.slot);
        Ok(())
    }

    /// Removes `node`. Its sidechain and modulation go at once; it keeps its outgoing connections
    /// while it rings out, if it has a tail. Returns the slots to retire now, the node and its
    /// chain, when nothing of it has a tail; the engine is told to end them when the revision
    /// takes effect. A processor taken out of an insert chain ends at once.
    ///
    /// # Errors
    ///
    /// [`AudioError::Master`], [`AudioError::StaleNode`], or [`AudioError::NodeInUse`] while
    /// connections reach its audio inputs.
    pub fn remove(&mut self, node: NodeKey) -> Result<Vec<NodeKey>, AudioError> {
        if node == NodeKey::MASTER {
            return Err(AudioError::Master);
        }
        self.check_live(node)?;
        if let Some(host) = self.nodes[node.slot as usize].host.take() {
            self.nodes[host as usize].inserts.retain(|slot| *slot != node.slot);
            self.links.retain(|link| link.to != node.slot);
            self.nodes[node.slot as usize].state = State::Removing;
            return Ok(vec![node]);
        }
        let feeding = self.feeds(node);
        if feeding > 0 {
            return Err(AudioError::NodeInUse {
                voices: 0,
                nodes: feeding,
            });
        }
        let mut gone = vec![node.slot];
        gone.extend(self.nodes[node.slot as usize].inserts.iter().copied());
        let ringing = self.tail(node.slot) > 0;
        for slot in &gone {
            self.links.retain(|link| link.to != *slot);
            let entry = &mut self.nodes[*slot as usize];
            entry.state = State::Removing;
            entry.ringing = ringing;
        }
        if ringing {
            return Ok(Vec::new());
        }
        for slot in &gone {
            self.links.retain(|link| link.from != *slot);
        }
        Ok(gone.iter().map(|slot| self.key(*slot)).collect())
    }

    /// The engine is done with `node`: its connections go, its slot is free, and the next node in
    /// it is a new one.
    pub fn ended(&mut self, node: NodeKey) {
        let entry = &mut self.nodes[node.slot as usize];
        if entry.generation != node.generation || entry.state == State::Free {
            return;
        }
        entry.generation = entry.generation.wrapping_add(1);
        entry.state = State::Free;
        entry.host = None;
        entry.inserts.clear();
        entry.route = None;
        entry.ringing = false;
        self.links.retain(|link| link.from != node.slot && link.to != node.slot);
        self.free.push(node.slot);
    }

    /// The node that runs `slot`'s steps: its host, or itself.
    fn unit(&self, slot: u32) -> u32 {
        self.nodes[slot as usize].host.unwrap_or(slot)
    }

    /// Whether a removed node still runs: ringing out, its chain with it.
    fn runs(&self, slot: u32) -> bool {
        let node = &self.nodes[slot as usize];
        match node.state {
            State::Free => false,
            State::Live => true,
            State::Removing => node.ringing,
        }
    }

    /// The longest tail of node `slot` and its chain.
    fn tail(&self, slot: u32) -> u64 {
        let node = &self.nodes[slot as usize];
        node.inserts
            .iter()
            .map(|insert| self.nodes[*insert as usize].shape.tail)
            .fold(node.shape.tail, u64::max)
    }

    /// Every node that runs and isn't in a chain, each before everything it feeds and the master
    /// last. Among nodes that could go next, the lowest slot goes first, so the same graph always
    /// runs in the same order.
    fn order(&self) -> Result<Vec<u32>, AudioError> {
        let running = |slot: u32| self.nodes[slot as usize].host.is_none() && self.runs(slot);
        let mut waiting = vec![0usize; self.nodes.len()];
        for link in self.links.iter().filter(|link| link.feedback == 0) {
            let (from, to) = (self.unit(link.from), self.unit(link.to));
            if !running(from) || !running(to) {
                continue;
            }
            if from == to {
                return Err(AudioError::Cycle);
            }
            waiting[to as usize] += 1;
        }
        let units: Vec<u32> = (0..self.nodes.len())
            .filter_map(|slot| u32::try_from(slot).ok())
            .filter(|slot| running(*slot))
            .collect();
        let mut ready: BTreeSet<u32> = units.iter().copied().filter(|slot| waiting[*slot as usize] == 0).collect();
        let mut order = Vec::with_capacity(units.len());
        while let Some(slot) = ready.pop_first() {
            order.push(slot);
            for link in self.links.iter().filter(|link| link.feedback == 0) {
                if self.unit(link.from) != slot || !running(self.unit(link.to)) {
                    continue;
                }
                let to = self.unit(link.to);
                waiting[to as usize] -= 1;
                if waiting[to as usize] == 0 {
                    ready.insert(to);
                }
            }
        }
        if order.len() < units.len() {
            return Err(AudioError::Cycle);
        }
        Ok(order)
    }

    /// The latency of the node in `slot`'s chain before it, and of the chain whole.
    fn chain_latency(&self, slot: u32) -> (u64, u64) {
        let unit = self.unit(slot);
        let mut before = 0;
        let mut whole = 0;
        for insert in &self.nodes[unit as usize].inserts {
            if *insert == slot {
                before = whole;
            }
            whole += self.nodes[*insert as usize].shape.latency;
        }
        (before, whole)
    }

    /// When each node's signal is where, by slot, along `order`.
    fn timing(&self, order: &[u32]) -> Vec<Timing> {
        let mut timing = vec![Timing::default(); self.nodes.len()];
        for &unit in order {
            let arrival = self
                .links
                .iter()
                .filter(|link| link.feedback == 0 && matches!(link.input, Input::Port(_)) && self.unit(link.to) == unit)
                .map(|link| source_time(&timing, link))
                .max()
                .unwrap_or(0);
            let (_, chain) = self.chain_latency(unit);
            let pre = arrival + chain;
            timing[unit as usize] = Timing {
                arrival,
                pre,
                post: pre + self.nodes[unit as usize].shape.latency,
            };
        }
        timing
    }

    /// How many frames `node`'s output lags the voices that reach it, by the longest way round.
    ///
    /// # Errors
    ///
    /// [`AudioError::StaleNode`].
    pub fn latency(&self, node: NodeKey) -> Result<u64, AudioError> {
        self.check_live(node)?;
        let order = self.order()?;
        let timing = self.timing(&order);
        Ok(timing[self.unit(node.slot) as usize].post)
    }

    /// Compiles the graph into a plan. See [`Plan`].
    ///
    /// # Errors
    ///
    /// [`AudioError::Cycle`] for a loop without a feedback delay on it.
    #[allow(clippy::too_many_lines)] // One pass per thing a plan holds, in the order they build.
    pub fn compile(&self) -> Result<Plan, AudioError> {
        let order = self.order()?;
        let timing = self.timing(&order);
        // Each unit's chain runs before it, in its buffer.
        let mut steps: Vec<u32> = Vec::with_capacity(order.len());
        for &unit in &order {
            steps.extend(self.nodes[unit as usize].inserts.iter().copied());
            steps.push(unit);
        }
        let mut position = vec![usize::MAX; self.nodes.len()];
        for (index, slot) in steps.iter().enumerate() {
            position[*slot as usize] = index;
        }
        let runs = |slot: u32| position[slot as usize] != usize::MAX;
        let wired: Vec<&Link> = self.links.iter().filter(|link| runs(link.from) && runs(link.to)).collect();

        // Scratch: each output other than a buffer, from its step to the last step that reads it.
        let mut last_read = vec![[0usize; crate::audio::processor::MAX_PORTS]; self.nodes.len()];
        for link in &wired {
            if matches!(link.input, Input::Param(_)) || self.is_sidechain(link) {
                let read = &mut last_read[link.from as usize][link.output as usize];
                *read = (*read).max(position[link.to as usize]);
            }
        }
        let mut free_after: Vec<usize> = Vec::new();
        let mut outputs = vec![Span::default(); self.nodes.len()];
        for (index, &slot) in steps.iter().enumerate() {
            let node = &self.nodes[slot as usize];
            if node.host.is_some() {
                continue;
            }
            let first = usize::from(node.shape.ports.in_place);
            let count = node.shape.ports.outputs.len() - first;
            let start = (0..=free_after.len())
                .find(|start| (*start..start + count).all(|stride| free_after.get(stride).is_none_or(|free| *free < index)))
                .unwrap_or(free_after.len());
            if free_after.len() < start + count {
                free_after.resize(start + count, 0);
            }
            for output in 0..count {
                free_after[start + output] = last_read[slot as usize][first + output].max(index);
            }
            outputs[slot as usize] = Span {
                start: to_u32(start),
                end: to_u32(start + count),
            };
        }
        let place = |slot: u32, output: u8| -> Place {
            let node = &self.nodes[slot as usize];
            if output == 0 && node.shape.ports.in_place {
                return Place::Buffer(slot);
            }
            let first = u32::from(node.shape.ports.in_place);
            Place::Scratch(outputs[slot as usize].start + u32::from(output) - first)
        };

        let mut lines: Vec<(Edge, u64, usize)> = Vec::new();
        let mut rings: Vec<(Edge, u64, usize)> = Vec::new();
        let mut pushes: Vec<(Push, Option<Edge>)> = Vec::new();
        let mut feedback: Vec<(Feedback, Edge)> = Vec::new();
        let mut planned: Vec<(Step, Option<Keyed>)> = Vec::with_capacity(steps.len());
        for &slot in &steps {
            let node = &self.nodes[slot as usize];
            let unit = self.unit(slot);
            let drain = (node.state == State::Removing).then(|| self.tail(unit));
            let mut step = Step {
                slot,
                generation: node.generation,
                buffer: unit,
                channels: to_u8(node.channels),
                separate: !node.shape.ports.in_place,
                outputs: outputs[slot as usize],
                pre: Span::default(),
                post: Span::default(),
                feedback: Span::default(),
                key: None,
                drain,
                metered: node.shape.mixer,
            };
            // What the node hears on its sidechain, as late as its own input arrives there.
            let key = wired
                .iter()
                .find(|link| link.to == slot && self.is_sidechain(link))
                .map(|link| {
                    let (before, _) = self.chain_latency(slot);
                    let wanted = timing[unit as usize].arrival + if node.host.is_some() { before } else { 0 };
                    let behind = wanted - source_time(&timing, link);
                    let edge = edge(link);
                    if behind > 0 {
                        lines.push((edge, behind, node.channels));
                    }
                    let pull = Pull {
                        from: place(link.from, link.output),
                        from_channels: to_u8(self.nodes[link.from as usize].channels),
                        line: None,
                    };
                    (pull, (behind > 0).then_some(edge))
                });
            if node.host.is_none() {
                for tap in [Tap::PreFader, Tap::PostFader] {
                    let start = to_u32(pushes.len());
                    for link in wired
                        .iter()
                        .filter(|link| link.from == slot && link.tap == tap && link.feedback == 0)
                    {
                        let Input::Port(port) = link.input else {
                            continue;
                        };
                        if self.is_sidechain(link) {
                            continue;
                        }
                        let sink = &self.nodes[link.to as usize];
                        let behind = timing[link.to as usize].arrival - source_time(&timing, link);
                        let edge = edge(link);
                        if behind > 0 {
                            lines.push((edge, behind, sink.channels));
                        }
                        let push = Push {
                            from: if tap == Tap::PreFader {
                                Place::Buffer(slot)
                            } else {
                                place(slot, link.output)
                            },
                            from_channels: to_u8(node.channels),
                            to: Target {
                                slot: link.to,
                                generation: sink.generation,
                                input: self.audio_index(link.to, port),
                            },
                            to_channels: to_u8(sink.channels),
                            level: link.level,
                            line: None,
                        };
                        pushes.push((push, (behind > 0).then_some(edge)));
                    }
                    let span = Span {
                        start,
                        end: to_u32(pushes.len()),
                    };
                    if tap == Tap::PreFader {
                        step.pre = span;
                    } else {
                        step.post = span;
                    }
                }
                let start = to_u32(feedback.len());
                for link in wired.iter().filter(|link| link.from == slot && link.feedback > 0) {
                    let Input::Port(port) = link.input else {
                        continue;
                    };
                    let sink = &self.nodes[link.to as usize];
                    let edge = edge(link);
                    rings.push((edge, link.feedback, sink.channels));
                    let entry = Feedback {
                        ring: 0,
                        from: place(slot, link.output),
                        from_channels: to_u8(node.channels),
                        to: Target {
                            slot: link.to,
                            generation: sink.generation,
                            input: self.audio_index(link.to, port),
                        },
                    };
                    feedback.push((entry, edge));
                }
                step.feedback = Span {
                    start,
                    end: to_u32(feedback.len()),
                };
            }
            planned.push((step, key));
        }

        // What voices add into a node waits as long as the latest path into it.
        let mut entries = Vec::new();
        for &unit in &order {
            let node = &self.nodes[unit as usize];
            let arrival = timing[unit as usize].arrival;
            if arrival == 0 || node.state != State::Live {
                continue;
            }
            for (index, port) in node.shape.ports.inputs.iter().enumerate() {
                if port.kind != PortKind::Audio {
                    continue;
                }
                let edge = Edge {
                    from: INPUT,
                    output: 0,
                    to: unit,
                    input: to_u8(index),
                };
                lines.push((edge, arrival, node.channels));
                let target = Target {
                    slot: unit,
                    generation: node.generation,
                    input: self.audio_index(unit, to_u8(index)),
                };
                entries.push((edge, target, to_u8(node.channels)));
            }
        }

        let delays = Delays::new(lines);
        let rings = Rings::new(rings);
        let line = |edge: Option<Edge>| edge.and_then(|edge| delays.find(edge));
        let steps = planned
            .into_iter()
            .map(|(mut step, key)| {
                step.key = key.map(|(pull, edge)| Pull {
                    line: line(edge),
                    ..pull
                });
                step
            })
            .collect();
        let pushes = pushes
            .into_iter()
            .map(|(push, edge)| Push {
                line: line(edge),
                ..push
            })
            .collect();
        let feedback = feedback
            .into_iter()
            .map(|(entry, edge)| Feedback {
                ring: rings.find(edge).unwrap_or(0),
                ..entry
            })
            .collect();
        let entries = entries
            .into_iter()
            .filter_map(|(edge, target, channels)| Some((delays.find(edge)?, target, channels)))
            .collect();
        let clear = order
            .iter()
            .map(|&unit| Clear {
                slot: unit,
                generation: self.nodes[unit as usize].generation,
                channels: to_u8(self.nodes[unit as usize].channels),
            })
            .collect();
        Ok(Plan {
            steps,
            pushes,
            feedback,
            clear,
            entries,
            delays,
            rings,
            scratch: vec![0.0; free_after.len() * self.channels * QUANTUM].into_boxed_slice(),
        })
    }

    fn is_sidechain(&self, link: &Link) -> bool {
        matches!(link.input, Input::Port(port)
            if self.nodes[link.to as usize].shape.ports.inputs[port as usize].kind == PortKind::Sidechain)
    }

    /// Input port `port` of node `slot` counted among its audio inputs only.
    fn audio_index(&self, slot: u32, port: u8) -> u8 {
        let inputs = self.nodes[slot as usize].shape.ports.inputs;
        to_u8(
            inputs[..port as usize]
                .iter()
                .filter(|known| known.kind == PortKind::Audio)
                .count(),
        )
    }
}

/// A sidechain pull, and the compensation line it takes if it needs one.
type Keyed = (Pull, Option<Edge>);

/// When the signal a link carries leaves its source.
fn source_time(timing: &[Timing], link: &Link) -> u64 {
    let source = timing[link.from as usize];
    if link.tap == Tap::PreFader { source.pre } else { source.post }
}

fn edge(link: &Link) -> Edge {
    let input = match link.input {
        Input::Port(port) => port,
        Input::Param(_) => u8::MAX,
    };
    Edge {
        from: link.from,
        output: link.output,
        to: link.to,
        input,
    }
}

fn to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn to_u8(value: usize) -> u8 {
    u8::try_from(value).unwrap_or(u8::MAX)
}
