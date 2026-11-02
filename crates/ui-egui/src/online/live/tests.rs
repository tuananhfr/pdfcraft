//! Two apps co-editing through a fake shell that keeps the lock table, grants or refuses locks
//! at once and relays each region to the other side, natively.

use std::cell::RefCell;
use std::rc::Rc;

use pdfcraft_engine::{ClientPrefix, DocId, Edit, EditError, FieldValue};

use super::*;
use crate::PdfCraftApp;
use crate::online::tests::{FORM, Fake, app, open, open_with, tick};

const NAMES: [&str; 2] = ["Ada", "Bea"];

#[derive(Default)]
pub(in crate::online) struct Room {
    inboxes: Vec<LiveInbox>,
    /// (region, seat)
    locks: Vec<(String, usize)>,
    /// What each seat called, in order.
    log: Vec<String>,
}

/// `*` clashes with every region, any other region only with itself.
fn clash(a: &str, b: &str) -> bool {
    a == b || a == "*" || b == "*"
}

impl Room {
    fn push(&self, seat: usize, event: LiveEvent) {
        self.inboxes[seat].lock().unwrap().push(event);
    }

    fn broadcast_locks(&self) {
        for seat in 0..self.inboxes.len() {
            let locks = self
                .locks
                .iter()
                .map(|(region, holder)| LockView {
                    region: region.clone(),
                    holder: format!("m{holder}"),
                    colour: Some("#e11d48".into()),
                    name: None,
                    mine: *holder == seat,
                })
                .collect();
            self.push(seat, LiveEvent::Locks(locks));
        }
    }
}

#[derive(Clone)]
struct Seat {
    room: Rc<RefCell<Room>>,
    me: usize,
}

impl RegionHost for Seat {
    fn region_will_edit(&self, region: &str, reply: Box<dyn FnOnce(bool)>) {
        let granted = {
            let mut room = self.room.borrow_mut();
            room.log.push(format!("{}: willEdit {region}", self.me));
            let free = !room.locks.iter().any(|(r, h)| *h != self.me && clash(r, region));
            if free && !room.locks.iter().any(|(r, h)| *h == self.me && r == region) {
                room.locks.push((region.to_owned(), self.me));
                room.broadcast_locks();
            }
            free
        };
        reply(granted);
    }
    fn region_changed(&self, region: &str, payload: &str) {
        let mut room = self.room.borrow_mut();
        room.log.push(format!("{}: changed {region}", self.me));
        for seat in (0..room.inboxes.len()).filter(|s| *s != self.me) {
            room.push(seat, LiveEvent::ApplyRemote { region: region.into(), payload: payload.into(), author: Some(format!("m{}", self.me)) });
        }
    }
    fn region_did_end_edit(&self, region: &str) {
        let mut room = self.room.borrow_mut();
        room.log.push(format!("{}: didEndEdit {region}", self.me));
        let me = self.me;
        room.locks.retain(|(r, h)| !(*h == me && r == region));
        room.broadcast_locks();
    }
    fn selection_changed(&self, region: &str, range: &str) {
        let room = self.room.borrow();
        for seat in (0..room.inboxes.len()).filter(|s| *s != self.me) {
            let cursor =
                Cursor { author: format!("m{}", self.me), region: region.into(), range: range.into(), name: None, colour: Some("#2563eb".into()) };
            room.push(seat, LiveEvent::Cursors(vec![cursor]));
        }
    }
}

pub(in crate::online) struct Client {
    pub(in crate::online) app: PdfCraftApp,
    fake: Fake,
    pub(in crate::online) id: DocId,
}

impl Client {
    pub(in crate::online) fn tick(&mut self, now: f64) {
        tick(&mut self.app, &self.fake, now);
        self.app.live_step();
    }

    pub(in crate::online) fn edit(&mut self, edit: Edit) -> bool {
        self.app.active = self.app.views.iter().position(|v| v.id == self.id);
        self.app.apply_edit(edit)
    }

    fn fill(&mut self, value: &str) -> bool {
        self.edit(Edit::SetFieldValue { name: "name".into(), value: FieldValue::Text(value.into()) })
    }

    fn value(&self) -> Vec<String> {
        self.app.session.get(self.id).unwrap().form.iter().find(|f| f.name == "name").unwrap().value.clone()
    }

    fn toast(&self) -> Option<&str> {
        self.app.toast.as_ref().map(|t| t.0.as_str())
    }

    fn live(&self) -> &Live {
        self.app.online.as_ref().unwrap().live.as_ref().unwrap()
    }
}

/// Two apps with form.pdf open as the same host document, in one room.
fn pair() -> (Client, Client, Rc<RefCell<Room>>) {
    pair_with(FORM)
}

/// Two apps with `bytes` open as the same host document, in one room.
pub(in crate::online) fn pair_with(bytes: &[u8]) -> (Client, Client, Rc<RefCell<Room>>) {
    let room = Rc::new(RefCell::new(Room::default()));
    let mut clients = Vec::new();
    for (me, prefix) in ["0000aa", "0000bb"].into_iter().enumerate() {
        let (mut app, fake) = app();
        let seat = Seat { room: room.clone(), me };
        app.online = Some(app.online.take().unwrap().with_live(Box::new(seat)));
        app.session.set_client_prefix(ClientPrefix::parse(prefix).unwrap());
        room.borrow_mut().inboxes.push(app.online.as_ref().unwrap().live_inbox().unwrap());
        let id = open_with(&mut app, "d1", bytes);
        clients.push(Client { app, fake, id });
    }
    for seat in 0..2 {
        let members = (0..2).map(|m| Member { id: format!("m{m}"), name: Some(NAMES[m].into()), colour: None, me: m == seat }).collect();
        room.borrow().push(seat, LiveEvent::Presence(members));
    }
    let mut b = clients.pop().unwrap();
    let mut a = clients.pop().unwrap();
    // The first frame after the open puts the document in live mode.
    a.tick(0.0);
    b.tick(0.0);
    log(&room);
    (a, b, room)
}

pub(in crate::online) fn log(room: &Rc<RefCell<Room>>) -> Vec<String> {
    std::mem::take(&mut room.borrow_mut().log)
}

/// The field editor open on `name` with `text` typed.
fn focus(name: &str, text: &str) -> crate::forms_ui::Focus {
    crate::forms_ui::Focus {
        name: name.into(),
        widget: 0,
        text: text.into(),
        picked: Vec::new(),
        request_focus: false,
        select_all: false,
        calendar: None,
        calendar_rect: None,
    }
}

#[test]
fn an_edit_takes_the_lock_sends_the_region_and_lets_it_go_once_left() {
    let (mut a, mut b, room) = pair();
    a.tick(0.0);
    b.tick(0.0);
    assert!(a.app.session.is_live(a.id) && b.app.session.is_live(b.id));
    assert!(a.fill("Ada"));
    a.tick(100.0);
    assert_eq!(log(&room), ["0: willEdit field:name", "0: changed field:name"]);
    b.tick(100.0);
    assert_eq!(b.value(), ["Ada"]);
    assert_eq!(b.app.session.undo(b.id), Err(EditError::NothingToUndo), "someone else's change isn't B's to undo");
    assert_eq!(b.value(), ["Ada"]);
    assert!(b.app.session.get(b.id).unwrap().dirty, "the leader saves; every side holds the change");
    a.tick(100.0 + IDLE_END_MS + 1.0);
    assert_eq!(log(&room), ["0: didEndEdit field:name"]);
    assert!(room.borrow().locks.is_empty());
}

#[test]
fn a_region_someone_else_holds_refuses_the_edit_and_names_them() {
    let (mut a, mut b, _room) = pair();
    assert!(a.fill("Ada"));
    a.tick(0.0);
    b.tick(0.0);
    assert!(!b.fill("Bea"), "refused before it changes anything");
    assert_eq!(b.value(), ["Ada"]);
    assert_eq!(b.toast(), Some("Ada is editing this part."));
    assert!(b.app.session.take_live_changes(b.id).is_empty());
    // Another field is free.
    assert!(b.edit(Edit::SetFieldValue { name: "city".into(), value: FieldValue::Text("Huế".into()) }));
}

#[test]
fn a_refused_lock_takes_the_change_back_and_keeps_the_holders_version() {
    let (mut a, mut b, room) = pair();
    assert!(a.fill("Ada"));
    assert!(b.fill("Bea"), "B hasn't heard of A's lock yet");
    a.tick(0.0);
    b.tick(0.0);
    assert_eq!(b.value(), ["Ada"], "A's version, not the one before B's change");
    assert_eq!(b.toast(), Some("Ada is editing this part."));
    assert_eq!(log(&room), ["0: willEdit field:name", "0: changed field:name", "1: willEdit field:name"]);
    a.tick(1.0);
    assert_eq!(a.value(), ["Ada"], "nothing of B's reached A");
    assert_eq!(b.app.session.undo(b.id), Err(EditError::NothingToUndo), "the refused step left B's history");
}

#[test]
fn a_refused_lock_on_a_change_nobody_overwrote_puts_the_old_value_back() {
    let (mut a, mut b, _room) = pair();
    let before = b.value();
    // A starts filling the field: its lock is taken before the first keystroke.
    a.app.views[0].forms.focus = Some(focus("name", ""));
    a.tick(0.0);
    assert!(b.fill("Bea"));
    b.tick(0.0);
    assert_eq!(b.value(), before);
    assert_eq!(b.toast(), Some("Ada is editing this part."));
}

#[test]
fn page_operations_reach_the_other_side_with_the_same_ids_both_ways() {
    let (mut a, mut b, _room) = pair();
    assert!(a.edit(Edit::InsertBlankPage { at: 1, width: 300.0, height: 400.0 }));
    assert!(a.edit(Edit::RotatePages { pages: vec![0], degrees: 90 }));
    a.tick(0.0);
    b.tick(0.0);
    let ids_a = a.app.session.page_ids(a.id);
    assert_eq!(ids_a.len(), 2);
    assert_eq!(b.app.session.page_ids(b.id), ids_a);
    assert_eq!(b.app.session.get(b.id).unwrap().info.pages[0].rotation, 90);
    assert!(!b.edit(Edit::MovePages { pages: vec![1], to: 0 }), "A still holds the page order");
    a.tick(IDLE_END_MS + 1.0);
    b.tick(IDLE_END_MS + 1.0);
    assert!(b.edit(Edit::MovePages { pages: vec![1], to: 0 }));
    b.tick(IDLE_END_MS + 2.0);
    a.tick(IDLE_END_MS + 2.0);
    let moved = vec![ids_a[1], ids_a[0]];
    assert_eq!(a.app.session.page_ids(a.id), moved);
    assert_eq!(b.app.session.page_ids(b.id), moved);
}

#[test]
fn undo_skips_a_step_someone_else_changed_since_and_never_reverts_them() {
    let (mut a, mut b, room) = pair();
    assert!(a.fill("Ada"));
    a.tick(0.0);
    b.tick(0.0);
    a.tick(IDLE_END_MS + 1.0);
    b.tick(IDLE_END_MS + 1.0);
    assert!(b.fill("Bea"));
    b.tick(IDLE_END_MS + 2.0);
    a.tick(IDLE_END_MS + 2.0);
    assert_eq!(a.value(), ["Bea"]);
    log(&room);
    a.app.undo();
    assert_eq!(a.value(), ["Bea"], "B's change stays");
    assert_eq!(a.toast(), Some("Skipped a change someone else has edited since."));
    a.tick(IDLE_END_MS + 3.0);
    assert!(log(&room).is_empty(), "nothing to send");
    // B's own step is still B's to undo.
    b.app.undo();
    b.tick(IDLE_END_MS + 4.0);
    a.tick(IDLE_END_MS + 4.0);
    assert_eq!(a.value(), ["Ada"]);
    assert_eq!(b.value(), ["Ada"]);
}

#[test]
fn others_locks_and_cursors_are_drawn_on_their_pages() {
    let (mut a, mut b, _room) = pair();
    a.tick(0.0);
    b.tick(0.0);
    let page = b.app.views[0].live_marks.iter().find(|m| m.rect.is_none()).cloned();
    assert_eq!(page.map(|m| (m.page, m.name, m.lock)), Some((0, "Ada".to_string(), false)), "A's page as a name tag");
    assert!(a.fill("Ada"));
    a.tick(1.0);
    b.tick(1.0);
    let lock = b.app.views[0].live_marks.iter().find(|m| m.lock).cloned().expect("A's lock outlined");
    assert_eq!((lock.page, lock.name.as_str(), lock.colour), (0, "Ada", egui::Color32::from_rgb(0xe1, 0x1d, 0x48)));
    assert!(lock.rect.is_some());
    assert!(a.app.views[0].live_marks.iter().all(|m| !m.lock), "one's own lock isn't drawn");
}

#[test]
fn the_selection_is_reported_throttled() {
    let (mut a, mut b, _room) = pair();
    a.tick(0.0);
    b.tick(0.0);
    let pid = a.app.session.page_ids(a.id)[0];
    assert_eq!(b.live().cursors.iter().map(|c| c.region.clone()).collect::<Vec<_>>(), [format!("page:{pid}")]);
    a.app.views[0].forms.focus = Some(focus("city", ""));
    assert!(a.app.live_step(), "waits for the throttle");
    b.tick(1.0);
    assert_eq!(b.live().cursors[0].region, format!("page:{pid}"));
    a.tick(SELECTION_MS + 1.0);
    b.tick(SELECTION_MS + 1.0);
    assert_eq!(b.live().cursors[0].region, "field:city");
}

#[test]
fn leadership_is_only_noted() {
    let (mut a, _b, room) = pair();
    room.borrow().push(0, LiveEvent::Leader(true));
    a.tick(0.0);
    assert!(a.live().is_leader());
}

#[test]
fn without_erp_host_v3_nothing_is_live() {
    let (mut app, fake) = app();
    let id = open(&mut app, "d1");
    tick(&mut app, &fake, 0.0);
    assert!(!app.live_step());
    assert!(!app.session.is_live(id));
    app.active = app.views.iter().position(|v| v.id == id);
    assert!(app.apply_edit(Edit::SetFieldValue { name: "name".into(), value: FieldValue::Text("Ada".into()) }));
    assert!(app.session.take_live_changes(id).is_empty());
    app.undo();
    assert_eq!(app.session.get(id).unwrap().form.iter().find(|f| f.name == "name").unwrap().value, Vec::<String>::new());
}

#[test]
fn a_conflict_asks_the_user_keep_mine_puts_this_sides_version_back_use_theirs_keeps_the_room_s() {
    let (mut a, mut b, room) = pair();
    assert!(a.fill("Ada"));
    a.tick(0.0);
    b.tick(0.0);
    a.tick(IDLE_END_MS + 1.0);
    b.tick(IDLE_END_MS + 1.0);
    assert!(b.fill("Bea"));
    b.tick(IDLE_END_MS + 2.0);
    a.tick(IDLE_END_MS + 2.0);
    b.tick(2.0 * IDLE_END_MS + 3.0);
    log(&room);
    let answered = Rc::new(RefCell::new(Vec::new()));
    let reply = |answered: &Rc<RefCell<Vec<Choice>>>| -> ChoiceReply {
        let answered = answered.clone();
        Box::new(move |c| answered.borrow_mut().push(c))
    };
    room.borrow().push(0, LiveEvent::Conflict { region: "field:name".into(), reply: reply(&answered) });
    a.tick(3.0 * IDLE_END_MS);
    assert_eq!(a.live().conflicts.len(), 1, "waits for the user");
    assert!(answered.borrow().is_empty());
    a.app.answer_conflict(Choice::Theirs);
    assert_eq!(a.value(), ["Bea"]);
    room.borrow().push(0, LiveEvent::Conflict { region: "field:name".into(), reply: reply(&answered) });
    a.tick(3.0 * IDLE_END_MS + 1.0);
    a.app.answer_conflict(Choice::Mine);
    assert_eq!(a.value(), ["Ada"], "this side's last sent version");
    a.tick(3.0 * IDLE_END_MS + 2.0);
    assert!(log(&room).is_empty(), "the shell sends it, not the app");
    assert_eq!(*answered.borrow(), [Choice::Theirs, Choice::Mine]);
    assert!(a.live().conflicts.is_empty());
}

#[test]
fn a_lost_lock_keeps_the_typed_text_and_says_so() {
    let (mut a, _b, room) = pair();
    a.app.views[0].forms.focus = Some(focus("name", "Ad"));
    a.tick(1.0);
    assert_eq!(log(&room), ["0: willEdit field:name"]);
    room.borrow().push(0, LiveEvent::LockDenied("field:name".into()));
    a.tick(2.0);
    assert_eq!(a.app.views[0].forms.focus.as_ref().map(|f| f.text.as_str()), Some("Ad"));
    assert_eq!(a.toast(), Some("Someone else is editing this part."));
}

/// A hidden tab draws no frame while the shell keeps calling: whole tables, and a region's
/// payload straight after another of the same region, replace the queued ones; nothing that
/// touches the document between is dropped or reordered.
#[test]
fn the_queue_keeps_only_what_still_matters() {
    let apply = |region: &str, payload: &str| LiveEvent::ApplyRemote { region: region.into(), payload: payload.into(), author: None };
    let mut q = Vec::new();
    for i in 0..1000 {
        enqueue(&mut q, apply("annot:nm:a", &i.to_string()));
        enqueue(&mut q, LiveEvent::Cursors(Vec::new()));
        enqueue(&mut q, LiveEvent::Locks(Vec::new()));
        enqueue(&mut q, LiveEvent::Presence(Vec::new()));
        enqueue(&mut q, LiveEvent::Leader(i % 2 == 0));
    }
    assert_eq!(q.len(), 5);
    enqueue(&mut q, apply("annot:nm:b", "1"));
    enqueue(&mut q, apply("annot:nm:a", "x"));
    enqueue(&mut q, LiveEvent::LockDenied("annot:nm:a".into()));
    enqueue(&mut q, apply("annot:nm:a", "y"));
    let applied: Vec<&str> = q
        .iter()
        .filter_map(|e| match e {
            LiveEvent::ApplyRemote { payload, .. } => Some(payload.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(applied, ["999", "1", "x", "y"]);
    assert!(matches!(q.last(), Some(LiveEvent::ApplyRemote { payload, .. }) if payload == "y"));
}
