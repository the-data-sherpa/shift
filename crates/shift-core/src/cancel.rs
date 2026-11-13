//! The user's request to stop the running review.

use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::{ToolServer, ToolSpec};

/// Shared between the Cancel review button and the reviewer. A cancel names
/// the review call it was pressed for and never applies to another one.
#[derive(Clone, Default)]
pub struct Cancel(Arc<Mutex<State>>);

#[derive(Default)]
struct State {
    /// The id of the review call that is running, or ran last.
    current: Option<String>,
    /// The id of the call the user last cancelled.
    cancelled: Option<String>,
}

impl Cancel {
    /// The review call `call_id` starts.
    pub fn begin(&self, call_id: &str) {
        self.0.lock().unwrap().current = Some(call_id.to_owned());
    }

    /// Ask the review call `call_id` to stop. It may not have reached shift
    /// yet, or may already be over; either way no other call is affected.
    pub fn cancel(&self, call_id: &str) {
        self.0.lock().unwrap().cancelled = Some(call_id.to_owned());
    }

    /// Whether the running review call has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        let state = self.0.lock().unwrap();
        state.current.is_some() && state.current == state.cancelled
    }
}

/// Tools whose every call is one cancellable review call.
pub struct Cancellable<T> {
    pub tools: T,
    pub cancel: Cancel,
}

impl<T: ToolServer> ToolServer for Cancellable<T> {
    fn tools(&self) -> Vec<ToolSpec> {
        self.tools.tools()
    }

    fn call(&self, name: &str, arguments: Value, call_id: &str) -> Result<String, String> {
        self.cancel.begin(call_id);
        self.tools.call(name, arguments, call_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cancel_applies_only_to_the_call_it_names() {
        let cancel = Cancel::default();
        let button = cancel.clone();
        assert!(!cancel.is_cancelled());

        // Pressed during the first call: it stays cancelled until it ends.
        cancel.begin("call-1");
        assert!(!cancel.is_cancelled());
        button.cancel("call-1");
        assert!(cancel.is_cancelled());

        // The next call is untouched, even by a late press for the first.
        cancel.begin("call-2");
        assert!(!cancel.is_cancelled());
        button.cancel("call-1");
        assert!(!cancel.is_cancelled());

        // A press that lands just before its call reaches shift still counts.
        button.cancel("call-3");
        assert!(!cancel.is_cancelled());
        cancel.begin("call-3");
        assert!(cancel.is_cancelled());
    }
}
