//! Apartment-local TSF composition ownership and asynchronous write sessions.

use std::cell::{Cell, RefCell};
use std::mem::ManuallyDrop;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::{Rc, Weak};

use ime_pinyin_engine::{EngineReply, Preedit};
use windows::core::{implement, Error, IUnknown, Interface, Ref, Result};
use windows::Win32::Foundation::E_FAIL;
use windows::Win32::UI::TextServices::{
    ITfComposition, ITfCompositionSink, ITfCompositionSink_Impl, ITfContext, ITfContextComposition,
    ITfDocumentMgr, ITfEditSession, ITfEditSession_Impl, ITfInsertAtSelection, ITfRange,
    ITfThreadMgrEventSink, ITfThreadMgrEventSink_Impl, TF_AE_NONE, TF_ANCHOR_END, TF_ES_ASYNC,
    TF_ES_READWRITE, TF_IAS_NO_DEFAULT_COMPOSITION, TF_IAS_QUERYONLY, TF_SELECTION,
    TF_SELECTIONSTYLE,
};

use crate::broker_client::WorkerEvent;
use crate::composition_lease::CompositionLease;
use crate::edit_queue::{EditQueue, EditTicket};
use crate::key_adapter::KeyAdapter;
use crate::runtime_adapter::HostEditGuard;
use crate::windows_impl::ComLifetime;

#[derive(Default)]
struct CompositionData {
    context: Option<ITfContext>,
    active: Option<CompositionLease<ITfComposition>>,
    edits: EditQueue<CompositionEdit>,
}

enum CompositionEdit {
    Show(String, HostEditGuard),
    Commit(String, HostEditGuard),
    Cancel(HostEditGuard),
}

#[derive(Clone)]
pub(super) struct CompositionController {
    pub(super) keys: Rc<KeyAdapter>,
    data: Rc<RefCell<CompositionData>>,
}

impl CompositionController {
    pub(super) fn new() -> Self {
        Self {
            keys: Rc::new(KeyAdapter::new()),
            data: Rc::new(RefCell::new(CompositionData::default())),
        }
    }

    /// Test and Actual callbacks both bind context before inspecting the engine.
    pub(super) fn bind_context(&self, context: Option<&ITfContext>, client_id: u32) -> bool {
        let previous = self.data.borrow().context.clone();
        let same = match (&previous, context) {
            (Some(old), Some(new)) => same_object(old, new),
            (None, None) => true,
            _ => false,
        };
        if !same {
            self.reset(client_id);
            self.data.borrow_mut().context = context.cloned();
        }
        context.is_some() && client_id != 0
    }

    /// Invalidate queued writes immediately; cleanup targets only the old range.
    pub(super) fn reset(&self, client_id: u32) {
        self.invalidate(client_id, true);
    }

    /// Transport loss invalidates input, not the identity of a focused context.
    /// Keep that binding so reconnect does not discard the first fresh key.
    pub(super) fn reset_transport(&self, client_id: u32) {
        self.invalidate(client_id, false);
    }

    fn invalidate(&self, client_id: u32, drop_context: bool) {
        self.keys.reset_input();
        let (context, composition) = {
            let mut data = self.data.borrow_mut();
            data.edits.invalidate();
            let context = if drop_context {
                data.context.take()
            } else {
                data.context.clone()
            };
            (context, data.active.take())
        };
        if let (Some(context), Some(composition)) = (context, composition) {
            let session: ITfEditSession = CancelCompositionSession {
                composition,
                _lifetime: ComLifetime::new(),
            }
            .into();
            // Cleanup may be refused during shutdown. Never retry a host write
            // against a newly focused context or treat the old state as valid.
            let _ = request_write(&context, client_id, &session);
        }
    }

    fn schedule_with_receipt(
        &self,
        context: &ITfContext,
        client_id: u32,
        reply: EngineReply,
        mut receipt: HostEditGuard,
    ) -> bool {
        let edit = if let Some(commit) = reply.commit {
            CompositionEdit::Commit(commit, receipt)
        } else {
            match reply.preedit {
                Preedit::Keep => {
                    receipt.applied();
                    return true;
                }
                Preedit::Show(text) => CompositionEdit::Show(text, receipt),
                Preedit::Hide => CompositionEdit::Cancel(receipt),
            }
        };
        let ticket = match self.data.borrow_mut().edits.push(edit) {
            Ok(ticket) => ticket,
            Err(()) => {
                // Drop the RefCell borrow before resetting or calling COM.
                return false;
            }
        };
        let Some(ticket) = ticket else {
            return true;
        };
        let session: ITfEditSession = CompositionEditSession {
            context: context.clone(),
            controller: self.clone(),
            ticket,
            _lifetime: ComLifetime::new(),
        }
        .into();
        if request_write(context, client_id, &session).is_err() {
            let mut data = self.data.borrow_mut();
            if data.edits.is_current(ticket) {
                data.edits.reject(ticket);
                return false;
            }
            // A reentrant boundary already invalidated this request. Its late
            // refusal must not reset the newly focused engine/queue.
        }
        true
    }

    pub(super) fn poll_broker(&self, client_id: u32) {
        for _ in 0..32 {
            let Some(event) = self.keys.poll_broker() else {
                break;
            };
            match event {
                WorkerEvent::Opened { epoch, identity } if epoch == self.keys.broker_epoch() => {
                    let transition = self
                        .keys
                        .runtime
                        .event(ime_runtime_core::input_lifecycle::Event::Opened(identity));
                    if !self.keys.execute(transition.action) {
                        self.reset_transport(client_id);
                    }
                }
                WorkerEvent::Finished {
                    epoch,
                    identity,
                    ticket,
                } if epoch == self.keys.broker_epoch() => {
                    let transition = self.keys.runtime.event(
                        ime_runtime_core::input_lifecycle::Event::Finished { identity, ticket },
                    );
                    if !self.keys.execute(transition.action) {
                        self.reset_transport(client_id);
                    }
                }
                WorkerEvent::View {
                    epoch,
                    identity,
                    ticket,
                    reply,
                    receipt,
                } if epoch == self.keys.broker_epoch() => {
                    if receipt.identity() != identity {
                        self.reset_transport(client_id);
                        continue;
                    }
                    let Some(receipt) = self.keys.runtime.admit(receipt, ticket, &reply) else {
                        continue;
                    };
                    let context = self.data.borrow().context.clone();
                    if !context.as_ref().is_some_and(|context| {
                        self.schedule_with_receipt(context, client_id, reply, receipt)
                    }) {
                        self.reset_transport(client_id);
                    }
                }
                WorkerEvent::Failed(epoch) if epoch == self.keys.broker_epoch() => {
                    self.reset_transport(client_id)
                }
                _ => {} // Dropping a stale commit receipt sends Rejected for its old identity.
            }
        }
        let transition = self
            .keys
            .runtime
            .event(ime_runtime_core::input_lifecycle::Event::Pump);
        if !self.keys.execute(transition.action) {
            self.reset_transport(client_id);
        }
    }

    pub(super) fn context_sink(&self, client_id: u32) -> ITfThreadMgrEventSink {
        ContextEventSink {
            data: Rc::downgrade(&self.data),
            keys: Rc::downgrade(&self.keys),
            client_id,
            _lifetime: ComLifetime::new(),
        }
        .into()
    }
}

fn same_object<T: Interface>(left: &T, right: &T) -> bool {
    match (left.cast::<IUnknown>(), right.cast::<IUnknown>()) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn request_write(context: &ITfContext, client_id: u32, session: &ITfEditSession) -> Result<()> {
    if client_id == 0 {
        return Err(Error::from(E_FAIL));
    }
    // SAFETY: interfaces belong to this apartment; client_id came from Activate.
    // TF_ES_ASYNC prevents the key/focus callback from waiting for a write lock.
    unsafe { context.RequestEditSession(client_id, session, TF_ES_ASYNC | TF_ES_READWRITE)? }.ok()
}

fn set_caret_at_end(
    context: &ITfContext,
    range: &ITfRange,
    cookie: u32,
    current: impl Fn() -> bool,
) -> Result<()> {
    // SAFETY: cookie is the context write cookie from DoEditSession. Clone keeps
    // the selection's collapsed anchors separate from the composition range.
    let caret = unsafe { range.Clone()? };
    unsafe { caret.Collapse(cookie, TF_ANCHOR_END)? };
    if !current() {
        return Ok(());
    }
    let mut selection = TF_SELECTION {
        range: ManuallyDrop::new(Some(caret)),
        style: TF_SELECTIONSTYLE {
            ase: TF_AE_NONE,
            fInterimChar: false.into(),
        },
    };
    let result = unsafe { context.SetSelection(cookie, std::slice::from_ref(&selection)) };
    // TF_SELECTION is an ABI carrier and does not drop its interface automatically.
    unsafe { ManuallyDrop::drop(&mut selection.range) };
    result
}

#[implement(ITfCompositionSink)]
struct CompositionSink {
    alive: Rc<Cell<bool>>,
    // The composition may retain its sink. A weak back-reference avoids a COM/Rc cycle.
    data: Weak<RefCell<CompositionData>>,
    keys: Weak<KeyAdapter>,
    _lifetime: ComLifetime,
}

impl ITfCompositionSink_Impl for CompositionSink_Impl {
    fn OnCompositionTerminated(&self, cookie: u32, composition: Ref<ITfComposition>) -> Result<()> {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            // Expire detached compositions retained by delayed cleanup too.
            self.alive.set(false);
            let (Some(data), Some(composition)) = (self.data.upgrade(), composition.as_ref())
            else {
                return;
            };
            let active = data.borrow().active.clone();
            if active.as_ref().is_some_and(|active| {
                Rc::ptr_eq(&active.alive, &self.alive) && same_object(&active.handle, composition)
            }) {
                let active = {
                    let mut data = data.borrow_mut();
                    data.edits.invalidate();
                    data.active.take()
                };
                let clear = active.is_some_and(|active| active.preedit_written);
                if let Some(keys) = self.keys.upgrade() {
                    keys.reset_input();
                }
                // This callback supplies a write cookie. External termination
                // cancels our preedit; normal commits already dropped ownership
                // and do not enter this branch.
                if clear {
                    if let Ok(range) = unsafe { composition.GetRange() } {
                        let _ = unsafe { range.SetText(cookie, 0, &[]) };
                    }
                }
            }
        }));
        Ok(())
    }
}

#[implement(ITfEditSession)]
struct CompositionEditSession {
    context: ITfContext,
    controller: CompositionController,
    ticket: EditTicket,
    _lifetime: ComLifetime,
}

impl CompositionEditSession {
    fn current(&self) -> bool {
        self.controller.data.borrow().edits.is_current(self.ticket)
    }

    fn apply(&self, cookie: u32, edit: CompositionEdit) -> Result<()> {
        let active = self.controller.data.borrow().active.clone();
        if active
            .as_ref()
            .is_some_and(|composition| !composition.is_live())
        {
            return Err(Error::from(E_FAIL));
        }
        match edit {
            CompositionEdit::Show(text, mut receipt) => {
                let composition = if let Some(active) = active {
                    active
                } else {
                    let insert: ITfInsertAtSelection = self.context.cast()?;
                    // Query the insertion range before touching selected host text.
                    let range =
                        unsafe { insert.InsertTextAtSelection(cookie, TF_IAS_QUERYONLY, &[])? };
                    let manager: ITfContextComposition = self.context.cast()?;
                    let alive = Rc::new(Cell::new(true));
                    let sink: ITfCompositionSink = CompositionSink {
                        alive: Rc::clone(&alive),
                        data: Rc::downgrade(&self.controller.data),
                        keys: Rc::downgrade(&self.controller.keys),
                        _lifetime: ComLifetime::new(),
                    }
                    .into();
                    if !self.current() {
                        return Ok(());
                    }
                    receipt.attempting();
                    let composition = unsafe { manager.StartComposition(cookie, &range, &sink)? };
                    let composition = CompositionLease {
                        handle: composition,
                        alive,
                        preedit_written: false,
                    };
                    if !self.current() || !composition.is_live() {
                        let _ = clear_composition(&composition, cookie);
                        if self.current() {
                            self.controller.data.borrow_mut().edits.reject(self.ticket);
                            self.controller.keys.reset_input();
                        }
                        return Ok(());
                    }
                    self.controller.data.borrow_mut().active = Some(composition.clone());
                    composition
                };
                let range = unsafe { composition.handle.GetRange()? };
                if !self.current() {
                    return Ok(());
                }
                if !composition.is_live() {
                    return Err(Error::from(E_FAIL));
                }
                receipt.attempting();
                unsafe { range.SetText(cookie, 0, &text.encode_utf16().collect::<Vec<_>>())? };
                if self.current() {
                    if let Some(active) = self.controller.data.borrow_mut().active.as_mut() {
                        active.preedit_written = true;
                    }
                    set_caret_at_end(&self.context, &range, cookie, || self.current())?;
                }
                if self.current() && composition.is_live() {
                    receipt.applied();
                }
            }
            CompositionEdit::Commit(text, mut receipt) => {
                let text: Vec<u16> = text.encode_utf16().collect();
                if let Some(composition) = active {
                    let range = unsafe { composition.handle.GetRange()? };
                    if !self.current() {
                        return Ok(());
                    }
                    if !composition.is_live() {
                        return Err(Error::from(E_FAIL));
                    }
                    // The range now belongs to a commit attempt. Focus or
                    // termination reentry must not erase the committed text.
                    self.controller.data.borrow_mut().active = None;
                    receipt.attempting();
                    unsafe { range.SetText(cookie, 0, &text)? };
                    let caret_result = if self.current() {
                        set_caret_at_end(&self.context, &range, cookie, || self.current())
                    } else {
                        Ok(())
                    };
                    // Ownership is detached before EndComposition notifies its sink.
                    let end_result = if composition.is_live() {
                        unsafe { composition.handle.EndComposition(cookie) }
                    } else {
                        Ok(())
                    };
                    caret_result.and(end_result)?;
                } else {
                    let insert: ITfInsertAtSelection = self.context.cast()?;
                    if !self.current() {
                        return Ok(());
                    }
                    receipt.attempting();
                    let range = unsafe {
                        insert.InsertTextAtSelection(
                            cookie,
                            TF_IAS_NO_DEFAULT_COMPOSITION,
                            &text,
                        )?
                    };
                    if self.current() {
                        set_caret_at_end(&self.context, &range, cookie, || self.current())?;
                    }
                }
                receipt.applied();
            }
            CompositionEdit::Cancel(mut receipt) => {
                // End ownership before the host can reenter OnCompositionTerminated.
                let composition = {
                    let mut data = self.controller.data.borrow_mut();
                    data.active.take()
                };
                if let Some(composition) = composition {
                    receipt.attempting();
                    clear_composition(&composition, cookie)?;
                }
                if self.current() {
                    receipt.applied();
                }
            }
        }
        Ok(())
    }

    fn drain(&self, cookie: u32) -> Result<()> {
        // Host reentrancy can enqueue more keys. Bound work in a single write lock.
        for _ in 0..32 {
            let edit = self.controller.data.borrow_mut().edits.pop(self.ticket);
            let Some(edit) = edit else {
                return Ok(());
            };
            self.apply(cookie, edit)?;
        }
        if self
            .controller
            .data
            .borrow_mut()
            .edits
            .pop(self.ticket)
            .is_some()
        {
            return Err(Error::from(E_FAIL));
        }
        Ok(())
    }
}

impl ITfEditSession_Impl for CompositionEditSession_Impl {
    fn DoEditSession(&self, cookie: u32) -> Result<()> {
        let result = catch_unwind(AssertUnwindSafe(|| self.drain(cookie)))
            .unwrap_or_else(|_| Err(Error::from(E_FAIL)));
        if result.is_err() && self.current() {
            self.controller.keys.reset_input();
            let composition = {
                let mut data = self.controller.data.borrow_mut();
                data.edits.reject(self.ticket);
                data.active.take()
            };
            if let Some(composition) = composition {
                let _ = clear_composition(&composition, cookie);
            }
        }
        result
    }
}

fn clear_composition(composition: &CompositionLease<ITfComposition>, cookie: u32) -> Result<()> {
    if !composition.is_live() {
        return Ok(());
    }
    // EndComposition is attempted even if clearing our own preedit is refused.
    let clear = if composition.can_clear() {
        unsafe { composition.handle.GetRange() }.and_then(|range| {
            if composition.can_clear() {
                unsafe { range.SetText(cookie, 0, &[]) }
            } else {
                Ok(())
            }
        })
    } else {
        Ok(())
    };
    let end = if composition.is_live() {
        unsafe { composition.handle.EndComposition(cookie) }
    } else {
        Ok(())
    };
    clear.and(end)
}

#[implement(ITfEditSession)]
struct CancelCompositionSession {
    composition: CompositionLease<ITfComposition>,
    _lifetime: ComLifetime,
}

impl ITfEditSession_Impl for CancelCompositionSession_Impl {
    fn DoEditSession(&self, cookie: u32) -> Result<()> {
        catch_unwind(AssertUnwindSafe(|| {
            clear_composition(&self.composition, cookie)
        }))
        .unwrap_or_else(|_| Err(Error::from(E_FAIL)))
    }
}

#[implement(ITfThreadMgrEventSink)]
struct ContextEventSink {
    data: Weak<RefCell<CompositionData>>,
    keys: Weak<KeyAdapter>,
    client_id: u32,
    _lifetime: ComLifetime,
}

impl ContextEventSink {
    fn reset(&self) {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            if let (Some(data), Some(keys)) = (self.data.upgrade(), self.keys.upgrade()) {
                CompositionController { data, keys }.reset(self.client_id);
            }
        }));
    }
}

impl ITfThreadMgrEventSink_Impl for ContextEventSink_Impl {
    fn OnInitDocumentMgr(&self, _document: Ref<ITfDocumentMgr>) -> Result<()> {
        Ok(())
    }
    fn OnUninitDocumentMgr(&self, document: Ref<ITfDocumentMgr>) -> Result<()> {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let context = self
                .data
                .upgrade()
                .and_then(|data| data.borrow().context.clone());
            if let Some(context) = context {
                match (unsafe { context.GetDocumentMgr() }, document.as_ref()) {
                    (Ok(owner), Some(document)) if !same_object(&owner, document) => {}
                    _ => self.reset(),
                }
            }
        }));
        Ok(())
    }
    fn OnSetFocus(
        &self,
        focused: Ref<ITfDocumentMgr>,
        previous: Ref<ITfDocumentMgr>,
    ) -> Result<()> {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            if !matches!((focused.as_ref(), previous.as_ref()), (Some(a), Some(b)) if same_object(a, b))
            {
                self.reset();
                if let (Some(data), Some(keys), Some(document)) =
                    (self.data.upgrade(), self.keys.upgrade(), focused.as_ref())
                {
                    if let Ok(context) = unsafe { document.GetTop() } {
                        CompositionController { data, keys }
                            .bind_context(Some(&context), self.client_id);
                    }
                }
            }
        }));
        Ok(())
    }
    fn OnPushContext(&self, context: Ref<ITfContext>) -> Result<()> {
        self.reset();
        if let (Some(data), Some(keys), Some(context)) =
            (self.data.upgrade(), self.keys.upgrade(), context.as_ref())
        {
            CompositionController { data, keys }.bind_context(Some(context), self.client_id);
        }
        Ok(())
    }
    fn OnPopContext(&self, _context: Ref<ITfContext>) -> Result<()> {
        self.reset();
        Ok(())
    }
}
