---
title: 'Audio Graph'
description: 'The architecture of jess::audio as a compiled signal graph: definitions, plans, and node state kept apart; revisions made whole on the control side; voices that never touch the graph; feedback, latency, modulation, and the processor contract.'
weight: 79
extra:
  kind: guide
---

A crowded game scene, a procedural instrument, an adaptive score, and a multitrack session are all compositions of the same machinery: nodes that make, change, and mix signal, joined by connections, run on one sample clock. This page is the contract that machinery keeps. [Audio Engine](@/docs/contributing/audio-engine.md) covers the render path's rules, time, voices, streams, and devices; this one covers the graph.

## Three things, owned apart

```text
control side        Definition      nodes, ports, connections, insert chains: what the
                                      application asked for. Cloned, edited, checked.
                    compile ->      Plan: an order of steps, the pushes and pulls between
                                      them, delay lines, scratch layout. Immutable once built.
render side         Node table      every node's processor and its parameter automation,
                                      by slot. Lives as long as the node, whatever the plan.
                    Plan            the one the engine runs; replaced whole, never edited.
```

The **definition** is the graph as the application describes it. Only the control side has it. The **plan** is what the definition compiles to: the order nodes run in and how signal gets between them. The **node table** holds each node's state: its processor (a filter's memory, a reverb's tank, an oscillator's phase) and its parameters' automation.

Keeping the three apart is what lets the graph change while it plays. A plan refers to nodes by slot and generation and owns none of them, so swapping one plan for another leaves every processor where it was. Connecting an unrelated pair of nodes recompiles the plan and touches no reverb, filter, compressor, oscillator, or ramp anywhere. A node's state goes only when the node does.

## Nodes and ports

A node is a [processor](#the-processor-contract) in a slot of the node table, named by a `NodeKey` (slot and generation, so a handle that outlives its node names nothing). A bus is a node too: a mixer, whose processing is its gain and mute and whose output is metered. The master is the mixer in slot 0, and its output is the context's.

A node declares its ports:

- **Audio inputs**, any number up to eight. Everything connected to one sums.
- **Sidechain inputs**: signal the node listens to but doesn't pass on, as a compressor follows a key. One connection each.
- **Outputs**, up to eight.
- **Parameters**, each also a control input a connection can modulate.

Each node has a channel count, the context's unless asked for fewer. A connection between different counts converts by one rule: equal counts pass through, mono goes to every channel, and many channels go to mono as their average. Any other pair (stereo into quad, say) is refused when the connection is made. Nothing drops a channel silently.

Capacity is a budget, not a shape. A context has room for so many nodes and so many connections. Any node may feed any number of others, and any input may sum any number of feeds, up to that room.

## Voices stay out of the graph

A voice is not a node. It is an entry in a fixed table that renders its source and adds the result into one or more node inputs: its outputs, each with a level. The plan doesn't mention voices. Starting, stopping, culling, or ending one, or a thousand, compiles nothing and changes no plan. This is the path a game's sound effects take every tick, and it costs what it did when buses were all there was.

A voice can play into any node with an audio input: a bus, or a filter that only that voice goes through. Its outputs change with a command of their own (`route`, `send`, `unsend`). Inside a patch, that command reaches the engine in the same command as the plan, so the voice moves when the graph does.

A voice is virtualized (culled) at its source: a culled voice adds nothing. Whatever it plays through carries on without it, so a filter, a delay, or a reverb after it rings out as though the voice had stopped.

## Revisions

Every change to the definition is a revision: make a node, connect, disconnect, insert, route, remove. A patch (`AudioContext:patch`) is one revision holding many changes. The control side makes a revision whole before anything is sent:

1. It clones the definition and applies every change to the clone.
2. It checks the clone: live handles, ports that exist and are of the right kind, channel counts that convert, no cycle without a delay, room in the budgets.
3. It compiles the clone into a plan.
4. It sends the plan, any new nodes' processors, and any voice reroutes as one command, stamped with the revision's frame.
5. Only if that command was queued does the clone become the definition.

A revision either takes effect whole on its frame, or it fails and leaves the definition and the playing graph as they were. Nothing is compiled on the render thread. The engine only swaps in a plan it was handed, and sends the old one back to be dropped on the control side.

New nodes' processors are installed in the node table as soon as the engine takes the command. A node does nothing until the plan it came with runs: a voice sent to it before then is not heard, and is from the revision's frame.

Revisions take effect in the order they were made. A revision can be stamped for a later frame, and plans for later revisions are compiled on top of earlier ones. So a revision may not be stamped earlier than one still waiting: the control side refuses it (`GraphOrder`), naming the frame of the one it would overtake. That keeps every plan a simple function of the revisions before it. Nothing pending is ever recompiled, and the engine never has to work out which plan is current. Commands that aren't revisions (voices, parameters, the listener) keep their own stamps and order, as before.

## The plan

A plan is a list of steps, one for each node that runs. Each step names its node, the buffer it runs in, the signal it pulls before it runs, and the pushes it makes afterwards. Compiling one:

1. **Orders the steps.** It sorts the dependencies (audio and sidechain connections, modulation, insert chains) with Kahn's algorithm, lowest slot first, so the same definition always runs in the same order. A cycle with no delay on it is refused.
2. **Places buffers.** Every node has one input buffer, which voices can add into at any time. That is why it is permanent: which voices play where is not the plan's business. A node that runs in place processes there, and its first output is that buffer. Any other output (an extra port, or the output of a node that doesn't run in place) gets scratch from a pool. Each piece of scratch lives from the step that writes it to the last step that reads it, so outputs no longer needed share space, and the pool is as large as the most outputs live at once.
3. **Plans the signal.** Audio connections are pushed: right after a node runs, its output is added into each input it feeds, at the connection's level, converted, through a compensation delay if it needs one. Sidechains and modulation are pulled, read when the node listening to them runs.
4. **Works out latency**, and allocates compensation and feedback delay lines (below).

Each block, the engine clears the input buffers, adds the voices in, runs the steps in order, and writes the master out.

### Insert chains are an optimization

`AudioNode:insert` puts a processor in a node's insert chain: a bus's effects, before its gain. In the definition, a chain on node N is shorthand for a graph: N's summed input goes through each inserted processor in turn and then into N's own processing. The compiler runs that sub-graph in place in N's buffer, with no copy between processors.

An inserted processor is an ordinary node. It takes the same processors, parameters, automation, modulation, and sidechains, and reports latency and tail the same way. A processor can also be made standalone (`AudioContext:node`) and connected like any other node. What the chain fixes is only where an inserted processor's main input comes from, which is what makes in-place execution correct. A test renders a bus with inserts and the explicit graph it stands for, and requires the same samples.

## Feedback

A cycle is allowed only through a feedback connection (`feedback = frames`), which delays what it carries by that many frames, at least one quantum (128). The cycle check and the plan's order ignore feedback connections. The delay is a ring in the plan, indexed by the absolute output frame. A step writes the output's frames as it renders them, and the input's read, at block start, comes from frames already written. That is guaranteed because no block is longer than a quantum.

So what a feedback loop does depends only on the frames, not on how a render was cut into blocks. It doesn't matter how big a device's buffers are, or where a command split a block. A test renders the same loop in blocks of 1, 13, 128, and 1000 frames and requires the same samples. When a later plan keeps the same feedback connection, its ring inherits the old one's history, so editing elsewhere doesn't break the loop.

Feedback is deliberate delay, so latency compensation leaves it alone. It is not part of the longest-path analysis, and no compensation line is put on it. The period of a loop is its feedback delay, plus the latency of the processors around it, plus any compensation delays on the forward connections inside it. `AudioNode:latency` reports a node's forward latency only.

## Latency compensation

Each node's latency is what its processor reports, plus what its insert chain adds. The compiler walks the plan's order. A node's arrival is the latest output of anything feeding it through an audio or sidechain connection, and its output is its arrival plus its own latency. Every connection into it whose source finished earlier is delayed by the difference. What voices add directly arrives at zero, so it is delayed by the whole arrival. Parallel chains, dry and wet paths, and a sidechain key therefore line up where they meet.

Compensation removes only latency. A delay processor's echo, or a feedback connection's delay, is the sound itself and is never compensated. Modulation is control signal and isn't compensated either.

Compensation lines are part of the plan, so they change with the revision that needs them, on its frame. A new plan's line takes on the history of the line it replaces. One that keeps its length plays on without a seam; one that grows plays silence where it has no history; one that shrinks skips its oldest frames.

## Parameters, automation, and modulation

Every parameter has a range, a default, a unit, and a rate:

- **Control rate**: while the parameter moves, the processor hears where it has got to on every frame of the clock that is a multiple of 32 (`CONTROL_QUANTUM`), however the render is cut; between them, the ramp moves on unheard. A step lands on its own frame. A filter's cutoff is control rate: recomputing its coefficients every sample would buy nothing audible.
- **Audio rate**: the processor gets one value per sample while the parameter moves, and the steady value otherwise. A mixer's gain, a gain processor's, and an oscillator's frequency are audio rate.

Automation is a parameter's own schedule: set now, set on a frame, ramp linearly or exponentially to a frame, or cancel (drop what is scheduled and hold where it is). It lives in the node table with the processor, not in the plan.

Modulation is a connection from a node's output into a parameter, at a depth. The source is mixed to mono, and the parameter's value is its automation plus depth times the source, clamped to the parameter's range. A value that isn't finite falls back to the automation alone. A control-rate parameter hears its modulation on the same 32-frame grid, and an audio-rate one every sample. Modulation orders the plan as any connection does (the source runs first), so an oscillator can drive a filter's cutoff or another oscillator's frequency without Luau setting anything every block.

## The processor contract

A processor is a Rust value implementing `jess::audio::Processor`. It declares its kind, its parameters, its ports, whether it runs in place, its latency, and its tail. It gets one call per block:

```rust
fn process(&mut self, io: &mut Io<'_>);
```

`Io` lends the block's buffers: the main buffer (the summed first input, which the processor turns into its first output in place), other inputs, other outputs, the sidechain if one is connected, and per-sample values for audio-rate parameters that move. A processor that can't work in place says so, and gets its first input to read and a separate first output to write. A processor's output depends only on its input frames and its parameters, never on how the frames are cut into calls: the engine calls it with blocks of any length up to a quantum, wherever a device's buffers, a script's offline renders, or the next command happen to fall. Every built-in processor is tested for it. A processor never allocates, locks, or blocks in `process`. It is made on the control side with everything it will need, and goes back there to be dropped.

Scripts make processors by kind, through a registry. The built-in kinds are registered in it. A host embedding jess adds its own kinds with `Registry::register` before it makes a runtime (`jess::lua::Options`). A kind registered that way is made, connected, automated, and modulated from Luau exactly as a built-in one is, and adding one changes nothing in the engine. Registration is static: the processor is Rust code linked into the host, so it works the same on platforms that can't load libraries. There is no binary plugin interface, and none is promised. Luau never runs in a processor: nothing a script writes runs on the render thread.

## Removing things

A node can be removed once nothing feeds it: no voice plays into it and no connection reaches its inputs. Its own outgoing connections go with it, and so do the processors in its insert chain. A removed node whose processors have a tail (a reverb's decay, a delay's echoes) keeps running, unfed, for that tail in the plan of the revision that removed it, then goes. The engine sends its processors back to be dropped, and its slot is free once the control side has heard. Plans name nodes by generation, so a plan still running never treats the node that takes over a slot as the one that left it.

## Ownership

A context belongs to whatever made it. In Luau, an offline context lives as long as a script holds it. An output lives until it's closed or its runtime ends, which drops the engine and every node in it. Node, voice, and connection handles name things in the context and own nothing. A handle that outlives what it named makes calls that do nothing or say it is stale; it never reaches the thing that took its slot.

## The bus API over the graph

A bus is a mixer node plus conveniences, and every one of them is a revision of the same graph:

| Call | Revision |
| --- | --- |
| `context:bus{ parent }` | Makes a mixer node and connects its output to `parent`'s input: its route. |
| `bus:route(parent)` | Replaces the route connection. |
| `bus:send(target, { level, tap })` | Connects the bus to `target` at `level`, from after its gain or, with `tap = 'pre'`, from before it. |
| `bus:insert(kind)` | Makes a processor node and puts it in the bus's insert chain. |
| `node:listen(key)` | Replaces whatever reaches the processor's sidechain with a connection from `key`. |
| `bus:remove()` | Removes the node. |

None of the conveniences is a second mixer. They are the patch a script would write by hand. Voices are the exception to "everything is a revision". `voice:route`, `voice:send`, and `voice:unsend` move a voice with a command of its own, and that compiles nothing.

## What it costs

A game's graph is mostly buses, and most of its work is voices played into them. Both paths are kept as short as they were:

- **The voices a block walks** are the live ones, kept in a list in slot order, not a table of every slot a context could fill: a context sized for thousands of voices that plays hundreds pays for hundreds.
- **A voice's outputs** are a fixed first output and a slice of sends, allocated only when it has sends. Each output is checked against a flat table of the nodes' generations and channels, not against the nodes themselves, so a voice into a bus touches only the bus's buffer.
- **A bus's step** runs the mixer's own gain and mute kernel straight on its buffer, from the automation the engine holds. That is the same function the `mixer` processor calls, so a bus costs no virtual call and no `Io`. The processor still defines what a bus does.
- **A node whose parameters hold still** skips automation and modulation entirely, and **a block with nothing ringing out** doesn't look for anything to retire.

`tools/bench` measures it. `audio` is a crowded scene; `audio-churn` and `audio-render` start and stop hundreds of one-shots a second, each on its own frame, which cuts blocks down to a few frames and makes per-block cost dominate. Against the bus mixer the graph replaced, built from the tree a commit before it and run interleaved on one machine, the crowded scene is a little faster and the churning ones much faster, mostly for walking only the live voices.

## Limits

These are deliberate, and each is checked:

- **Feedback** needs a delay of at least a quantum (128 frames). There is no sample-by-sample feedback.
- **Revisions** take effect in the order they're made. A revision stamped earlier than one still waiting is refused (`GraphOrder`), not reordered.
- **Channel counts** go from 1 up to the context's, and connect only one way: the same count, mono into any, or any into mono as their average.
- **Every node keeps an input buffer**, because a voice may play into any node at any time. Only outputs other than a node's buffer share scratch.
- **Control-rate parameters** hear a ramp or a modulation every 32 frames, on a grid fixed to the clock: exact to the grid, not to the sample. A step is exact to the sample.
- **Modulation** is mixed to mono and isn't latency-compensated.
- **A processor has at most one sidechain**, and a sidechain hears one connection. A processor has at most eight ports and 64 parameters, and its latency is fixed for its life.
- **An inserted processor** takes no audio connections of its own and feeds nothing directly: its input and output are its host's chain.
- **A voice** plays into at most sixteen node inputs. It is culled at its source, so the nodes after it keep running, on silence.
- **A stream** seeks only while a feed writes it. Its position waits during a seek until the feed has decoded from the new frame.
- **Commands a cancel drops** keep counting against the schedule's room until their frames pass.
- **There is no binary plugin interface.** A processor from outside jess is Rust registered by the host that links it.

## What comes next, without replacing this

- **Capture** is a source node whose processor reads a device's input ring, filled by miniaudio's duplex devices. A recording is a stream a node writes and a script reads. Neither needs a change to plans or steps.
- **Multichannel output** is a context of up to eight channels. Wider buses and the conversion rules carry it, and a surround panner is a processor with an output per speaker.
- **Synthesis** is more processors (wavetables, noise, filters with audio-rate cutoff) and more modulation sources. MIDI is a Luau schedule of `set` and `gate` commands.
- **Third-party DSP** is the registry. A restricted, deterministic DSP language could later register processors the same way, compiled before they reach the render thread.
