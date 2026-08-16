//! Thread-safe delivery of graph edits into the audio thread.
//!
//! The editor cannot mutate the running [`RtEngine`] in place: the audio thread is
//! reading it, and rebuilding routing tables allocates. Instead the edit thread
//! *rebuilds* an engine off the audio thread and hands the finished engine over through
//! [`EngineSwap`], a lock-free pair of single-element mailboxes:
//!
//! ```text
//! editor thread                        audio thread
//! ─────────────                        ────────────
//! compile new RtEngine (allocates)
//! EngineSender::install ──▶ pending ──▶ EngineReceiver::poll_swap (atomic swap,
//! EngineSender::collect ◀── retired ◀── old engine handed back, never dropped here)
//!         │
//!         └─ drop old engine (deallocates, editor side)
//! ```
//!
//! The audio thread's whole interaction is two atomic pointer swaps per block — no
//! locks, no allocation, no deallocation. If the editor has not yet collected the
//! previously retired engine, the audio thread simply *defers* picking up the pending
//! one, so it never has to hold or free more than one engine.

use std::ptr;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::Arc;

use crate::engine::RtEngine;

/// Shared state between the two endpoints: two single-element pointer mailboxes.
struct Shared {
    /// Engine waiting to be picked up by the audio thread (editor → audio).
    pending: AtomicPtr<RtEngine>,
    /// Engine retired by the audio thread, awaiting drop (audio → editor).
    retired: AtomicPtr<RtEngine>,
}

impl Shared {
    fn new() -> Self {
        Self {
            pending: AtomicPtr::new(ptr::null_mut()),
            retired: AtomicPtr::new(ptr::null_mut()),
        }
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        // Reclaim whatever is still parked in the mailboxes when both endpoints are
        // gone. `get_mut` proves exclusive access, so no other thread can race us.
        for slot in [&mut self.pending, &mut self.retired] {
            let p = *slot.get_mut();
            if !p.is_null() {
                // SAFETY: the pointer came from Box::into_raw in install()/poll_swap()
                // and ownership was left in the mailbox; we are the last owner.
                drop(unsafe { Box::from_raw(p) });
            }
        }
    }
}

/// Editor-side endpoint: installs freshly compiled engines and frees retired ones.
pub struct EngineSender {
    shared: Arc<Shared>,
}

/// Audio-side endpoint: picks up pending engines at block boundaries.
pub struct EngineReceiver {
    shared: Arc<Shared>,
}

/// Creates a connected sender/receiver pair.
#[must_use]
pub fn engine_swap() -> (EngineSender, EngineReceiver) {
    let shared = Arc::new(Shared::new());
    (
        EngineSender {
            shared: Arc::clone(&shared),
        },
        EngineReceiver { shared },
    )
}

impl EngineSender {
    /// Makes `engine` the next engine the audio thread will run.
    ///
    /// If a previous installation was never picked up, it is replaced and dropped
    /// here, on the editor thread.
    pub fn install(&self, engine: RtEngine) {
        let new = Box::into_raw(Box::new(engine));
        let old = self.shared.pending.swap(new, Ordering::AcqRel);
        if !old.is_null() {
            // SAFETY: `old` was created by Box::into_raw in a previous install() and
            // the audio thread never saw it (we just swapped it out of the mailbox).
            drop(unsafe { Box::from_raw(old) });
        }
    }

    /// Frees the engine most recently retired by the audio thread, if any.
    ///
    /// Returns `true` when something was collected. Call this from the editor's idle
    /// loop; the audio thread defers further swaps until the retired slot is empty.
    pub fn collect(&self) -> bool {
        let old = self.shared.retired.swap(ptr::null_mut(), Ordering::AcqRel);
        if old.is_null() {
            return false;
        }
        // SAFETY: `old` was parked by the audio thread's poll_swap(), which
        // relinquished ownership to this mailbox.
        drop(unsafe { Box::from_raw(old) });
        true
    }
}

impl EngineReceiver {
    /// Swaps in a pending engine, if one is ready and the retired slot is free.
    ///
    /// Call at a block boundary on the audio thread. On a successful swap `current`
    /// becomes the new engine and the old engine is parked for the editor to free;
    /// returns `true`. Performs no allocation, no deallocation and takes no locks.
    pub fn poll_swap(&self, current: &mut RtEngine) -> bool {
        // Defer while the editor has not collected the previous retiree: completing a
        // swap now would leave us holding an engine we are not allowed to drop.
        if !self.shared.retired.load(Ordering::Acquire).is_null() {
            return false;
        }

        let incoming = self.shared.pending.swap(ptr::null_mut(), Ordering::AcqRel);
        if incoming.is_null() {
            return false;
        }

        // SAFETY: `incoming` came from Box::into_raw in install(); swapping it out of
        // the mailbox transferred ownership to us.
        let mut incoming = unsafe { Box::from_raw(incoming) };
        std::mem::swap(current, &mut *incoming);
        // `incoming` now holds the *old* engine. Park it instead of dropping: the
        // deallocation must happen on the editor thread. The retired slot was observed
        // empty above and only this thread stores into it, so the swap target is null.
        let prev = self
            .shared
            .retired
            .swap(Box::into_raw(incoming), Ordering::AcqRel);
        debug_assert!(prev.is_null(), "retired slot was checked empty at entry");
        true
    }
}

// No unsafe Send/Sync impls are needed: `AtomicPtr` is always Send + Sync, and the
// engines exchanged through it are Send because `Processor: Send` is part of the
// processing contract.

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use vb_core::{Graph, GraphNode, NodeId, Port, ProcessContext, Processor, SignalType};

    use super::engine_swap;
    use crate::engine::RtEngine;

    struct Constant(f32);
    impl Processor for Constant {
        fn process(&mut self, _ctx: &ProcessContext, _in: &[&[f32]], out: &mut [&mut [f32]]) {
            out[0].fill(self.0);
        }
    }

    fn const_engine(value: f32) -> (RtEngine, NodeId) {
        let mut g = Graph::new();
        let src = g.add_node(GraphNode::new(
            "const",
            vec![],
            vec![Port::output("out", SignalType::Audio)],
        ));
        let mut procs: HashMap<NodeId, Box<dyn Processor>> = HashMap::new();
        procs.insert(src, Box::new(Constant(value)));
        (RtEngine::compile(&g, procs, 4).unwrap(), src)
    }

    fn ctx() -> ProcessContext {
        ProcessContext {
            sample_rate: 48_000.0,
            frames: 4,
        }
    }

    #[test]
    fn poll_without_install_is_a_no_op() {
        let (_tx, rx) = engine_swap();
        let (mut engine, src) = const_engine(1.0);
        assert!(!rx.poll_swap(&mut engine));
        engine.process(&ctx());
        assert_eq!(
            engine.node_output(src).unwrap().channel(0).unwrap(),
            &[1.0; 4]
        );
    }

    #[test]
    fn installed_engine_is_swapped_in() {
        let (tx, rx) = engine_swap();
        let (mut engine, src) = const_engine(1.0);
        let (replacement, _) = const_engine(2.0);

        tx.install(replacement);
        assert!(rx.poll_swap(&mut engine));
        engine.process(&ctx());
        assert_eq!(
            engine.node_output(src).unwrap().channel(0).unwrap(),
            &[2.0; 4]
        );

        // The old engine is parked for the editor; collecting frees it.
        assert!(tx.collect());
        assert!(!tx.collect(), "retired slot should now be empty");
    }

    #[test]
    fn swap_defers_until_retired_engine_is_collected() {
        let (tx, rx) = engine_swap();
        let (mut engine, src) = const_engine(1.0);

        tx.install(const_engine(2.0).0);
        assert!(rx.poll_swap(&mut engine), "first swap goes through");

        tx.install(const_engine(3.0).0);
        assert!(
            !rx.poll_swap(&mut engine),
            "second swap defers: retiree not collected"
        );
        engine.process(&ctx());
        assert_eq!(
            engine.node_output(src).unwrap().channel(0).unwrap(),
            &[2.0; 4],
            "still on the second engine while deferred"
        );

        assert!(tx.collect());
        assert!(rx.poll_swap(&mut engine), "swap proceeds after collect");
        engine.process(&ctx());
        assert_eq!(
            engine.node_output(src).unwrap().channel(0).unwrap(),
            &[3.0; 4]
        );
    }

    #[test]
    fn reinstall_before_pickup_replaces_pending() {
        let (tx, rx) = engine_swap();
        let (mut engine, src) = const_engine(1.0);

        tx.install(const_engine(2.0).0);
        tx.install(const_engine(3.0).0); // replaces the unseen 2.0 engine
        assert!(rx.poll_swap(&mut engine));
        engine.process(&ctx());
        assert_eq!(
            engine.node_output(src).unwrap().channel(0).unwrap(),
            &[3.0; 4]
        );
    }

    #[test]
    fn cross_thread_smoke() {
        let (tx, rx) = engine_swap();
        let (mut engine, src) = const_engine(0.0);

        let audio = std::thread::spawn(move || {
            let ctx = ctx();
            // Run "blocks" until we see the final engine's value, with a hard cap so a
            // regression fails instead of hanging.
            for _ in 0..100_000 {
                rx.poll_swap(&mut engine);
                engine.process(&ctx);
                let sample = engine.node_output(src).unwrap().channel(0).unwrap()[0];
                if (sample - 5.0).abs() < f32::EPSILON {
                    return true;
                }
            }
            false
        });

        for value in [1.0, 2.0, 3.0, 4.0, 5.0] {
            tx.install(const_engine(value).0);
            std::thread::yield_now();
        }
        // Keep servicing the retired mailbox until the audio thread is done — swaps
        // defer while a retiree is uncollected, so the editor must keep collecting.
        while !audio.is_finished() {
            tx.collect();
            std::thread::yield_now();
        }
        assert!(
            audio.join().unwrap(),
            "audio thread never observed the final engine"
        );
    }
}
