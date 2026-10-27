//! The saves on their way to the shell, shared by the app's frame and the page's `flush()`
//! handler: one in flight per server document (rule 2), a later one waiting behind it, and the
//! answers kept for the next frame to apply.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use super::{Host, Reply, SaveReason, SaveResult};

/// A save to send.
#[derive(Clone, Debug)]
pub(super) struct Outgoing {
    pub doc_id: String,
    pub slot: u64,
    pub revision: u64,
    pub bytes: Arc<Vec<u8>>,
    pub reason: SaveReason,
}

/// A save the shell answered, waiting for the next frame.
#[derive(Debug)]
pub(super) struct Settled {
    pub slot: u64,
    pub revision: u64,
    pub bytes: Arc<Vec<u8>>,
    pub result: SaveResult,
}

struct Waiter {
    docs: Vec<String>,
    done: Box<dyn FnOnce()>,
}

#[derive(Default)]
struct State {
    /// Per server document: the slot and revision of the save in flight.
    in_flight: HashMap<String, (u64, u64)>,
    /// What `flush` asked for while a save of the document was in flight: sent once it ends.
    follow_ups: HashMap<String, Outgoing>,
    settled: Vec<Settled>,
    /// What `flush` sends, as of the end of the last frame.
    flushable: Vec<Outgoing>,
    waiters: Vec<Waiter>,
}

#[derive(Clone)]
pub(super) struct Wire {
    host: Rc<dyn Host>,
    state: Rc<RefCell<State>>,
}

impl Wire {
    pub fn new(host: Rc<dyn Host>) -> Self {
        Self { host, state: Rc::default() }
    }

    /// A save of `doc_id` is in flight or waiting behind one.
    pub fn busy(&self, doc_id: &str) -> bool {
        self.state.try_borrow().map_or(true, |s| s.in_flight.contains_key(doc_id) || s.follow_ups.contains_key(doc_id))
    }

    /// Send `out` now, or after the save of its document in flight (replacing an older wait).
    /// The same revision of the same opening already in flight is not sent twice.
    pub fn dispatch(&self, out: Outgoing) {
        let send = {
            let Ok(mut s) = self.state.try_borrow_mut() else { return };
            match s.in_flight.get(&out.doc_id) {
                Some(&(slot, revision)) if slot == out.slot && revision == out.revision => None,
                Some(_) => {
                    s.follow_ups.insert(out.doc_id.clone(), out);
                    None
                }
                None => {
                    s.in_flight.insert(out.doc_id.clone(), (out.slot, out.revision));
                    Some(out)
                }
            }
        };
        // No borrow is held here: the host may answer before `save` returns.
        if let Some(out) = send {
            self.send(out);
        }
    }

    fn send(&self, out: Outgoing) {
        let wire = self.clone();
        let (doc_id, slot, revision, bytes) = (out.doc_id.clone(), out.slot, out.revision, out.bytes.clone());
        let reply: Reply = Box::new(move |result| wire.answered(doc_id, Settled { slot, revision, bytes, result }));
        self.host.save(&out.doc_id, &out.bytes, out.revision, out.reason, reply);
    }

    fn answered(&self, doc_id: String, settled: Settled) {
        let next = {
            let Ok(mut s) = self.state.try_borrow_mut() else { return };
            s.in_flight.remove(&doc_id);
            // A conflict or a refusal stops sending the document: the shell decides what follows.
            let next = s.follow_ups.remove(&doc_id).filter(|_| settled.result.keeps_saving());
            s.settled.push(settled);
            next
        };
        if let Some(next) = next {
            self.dispatch(next);
        }
        self.wake_waiters();
    }

    /// The answers since the last call.
    pub fn take_settled(&self) -> Vec<Settled> {
        self.state.try_borrow_mut().map(|mut s| std::mem::take(&mut s.settled)).unwrap_or_default()
    }

    pub fn set_flushable(&self, items: Vec<Outgoing>) {
        if let Ok(mut s) = self.state.try_borrow_mut() {
            s.flushable = items;
        }
    }

    pub fn flush(&self, done: Box<dyn FnOnce()>) {
        let items = self.state.try_borrow().map(|s| s.flushable.clone()).unwrap_or_default();
        let docs = items.iter().map(|o| o.doc_id.clone()).collect();
        for item in items {
            self.dispatch(Outgoing { reason: SaveReason::Flush, ..item });
        }
        if let Ok(mut s) = self.state.try_borrow_mut() {
            s.waiters.push(Waiter { docs, done });
        }
        self.wake_waiters();
    }

    fn wake_waiters(&self) {
        let ready = {
            let Ok(mut s) = self.state.try_borrow_mut() else { return };
            let (ready, waiting): (Vec<Waiter>, Vec<Waiter>) = std::mem::take(&mut s.waiters)
                .into_iter()
                // Handed over once nothing waits behind a save in flight; the answers are the shell's.
                .partition(|w| w.docs.iter().all(|d| !s.follow_ups.contains_key(d)));
            s.waiters = waiting;
            ready
        };
        for waiter in ready {
            (waiter.done)();
        }
    }
}
