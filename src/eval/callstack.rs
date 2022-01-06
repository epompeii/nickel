//! In a lazy language like Nickel, there are no well delimited stack frames due to how function
//! application is evaluated. Additional information about the history of function calls is thus
//! stored in a call stack solely for better error reporting.
use super::IdentKind;
use crate::{
    identifier::Ident,
    position::{RawSpan, TermPos},
};
use codespan::FileId;

/// A call stack, saving the history of function calls.
#[derive(PartialEq, Clone, Default, Debug)]
pub struct CallStack(pub Vec<StackElem>);

/// Basic description of a function call. Used for error reporting.
pub struct CallDescr {
    /// The name of the called function, if any.
    pub head: Option<Ident>,
    /// The position of the application.
    pub span: RawSpan,
}

/// A call stack element.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum StackElem {
    /// A function body was entered. The position is the position of the original application.
    Fun(TermPos),
    /// An application was evaluated.
    App(TermPos),
    /// A variable was entered.
    Var {
        kind: IdentKind,
        id: Ident,
        pos: TermPos,
    },
    /// A record field was entered.
    Field {
        id: Ident,
        pos_record: TermPos,
        pos_field: TermPos,
        pos_access: TermPos,
    },
}

impl CallStack {
    pub fn new() -> Self {
        CallStack(Vec::new())
    }

    /// Push a marker to indicate that a var was entered.
    pub fn enter_var(&mut self, kind: IdentKind, id: Ident, pos: TermPos) {
        self.0.push(StackElem::Var { kind, id, pos });
    }

    /// Push a marker to indicate that an application was entered.
    pub fn enter_app(&mut self, pos: TermPos) {
        // We ignore application without positions, which have been generated by the interpreter.
        if pos.is_def() {
            self.0.push(StackElem::App(pos));
        }
    }

    /// Push a marker to indicate that during the evaluation an application, the function part was
    /// finally evaluated to an expression of the form `fun x => body`, and that the body of this
    /// function was entered.
    pub fn enter_fun(&mut self, pos: TermPos) {
        // We ignore application without positions, which have been generated by the interpreter.
        if pos.is_def() {
            self.0.push(StackElem::Fun(pos));
        }
    }

    /// Push a marker to indicate that a record field was entered.
    pub fn enter_field(
        &mut self,
        id: Ident,
        pos_record: TermPos,
        pos_field: TermPos,
        pos_access: TermPos,
    ) {
        self.0.push(StackElem::Field {
            id,
            pos_record,
            pos_field,
            pos_access,
        });
    }

    /// Process a raw callstack by aggregating elements belonging to the same call. Return a list
    /// of call descriptions from the most nested/recent to the least nested/recent, together with
    /// the last pending call, if any.
    ///
    /// Recall that when a call `f arg` is evaluated, the following events happen:
    /// 1. `arg` is pushed on the evaluation stack.
    /// 2. `f` is evaluated.
    /// 3. Hopefully, the result of this evaluation is a function `Func(id, body)`. `arg` is popped
    ///    from the stack, bound to `id` in the environment, and `body is entered`.
    ///
    /// For error reporting purpose, we want to be able to determine the chain of nested calls leading
    /// to the current code path at any moment. To do so, the Nickel abstract machine maintains a
    /// callstack via this basic mechanism:
    /// 1. When an application is evaluated, push a marker with the position of the application on the callstack.
    /// 2. When a function body is entered, push a marker with the position of the original application on the
    ///    callstack.
    /// 3. When a variable is evaluated, push a marker with its name and position on the callstack.
    /// 4. When a record field is accessed, push a marker with its name and position on the
    ///    callstack too.
    ///
    /// Both field and variable are useful to determine the name of a called function, when there
    /// is one.  The resulting stack is not suited to be reported to the user for the following
    /// reasons:
    ///
    /// 1. One call spans several items on the callstack. First the application is entered (pushing
    ///    an `App`), then possibly variables or other application are evaluated until we
    ///    eventually reach a function for the left hand side. Then body of this function is
    ///    entered (pushing a `Fun`).
    /// 2. Because of currying, multi-ary applications span several objects on the callstack.
    ///    Typically, `(fun x y => x + y) arg1 arg2` spans two `App` and two `Fun` elements in the
    ///    form `App1 App2 Fun2 Fun1`, where the position span of `App1` includes the position span
    ///    of `App2`.  We want to group them as one call.
    /// 3. The callstack includes calls to builtin contracts. These calls are inserted implicitly
    ///    by the abstract machine and are not written explicitly by the user. Showing them is
    ///    confusing and clutters the call chain, so we get rid of them too.
    ///
    /// This is the role of `group_by_calls`, which filter out unwanted elements and groups
    /// callstack elements into atomic call elements represented by [`CallDescr`].
    ///
    /// The final call description list is reversed such that the most nested calls, which are
    /// usually the most relevant to understand the error, are printed first.
    ///
    /// # Arguments
    ///
    /// - `contract_id`: the `FileId` of the source containing standard contracts, to filter their
    ///   calls out.
    pub fn group_by_calls(
        self: &CallStack,
        contract_id: FileId,
    ) -> (Vec<CallDescr>, Option<CallDescr>) {
        // We filter out calls and accesses made from within the builtin contracts, as well as
        // generated variables introduced by program transformations.
        let it = self.0.iter().filter(|elem| match elem {
            StackElem::Var {id, ..} if id.is_generated() => false,
            StackElem::Var{ pos: TermPos::Original(RawSpan { src_id, .. }), ..}
            | StackElem::Var{pos: TermPos::Inherited(RawSpan { src_id, .. }), ..}
            | StackElem::Fun(TermPos::Original(RawSpan { src_id, .. }))
            | StackElem::Field {pos_access: TermPos::Original(RawSpan { src_id, .. }), ..}
            | StackElem::Field {pos_access: TermPos::Inherited(RawSpan { src_id, .. }), ..}
            | StackElem::App(TermPos::Original(RawSpan { src_id, .. }))
            // We avoid applications (Fun/App) with inherited positions. Such calls include
            // contracts applications which add confusing call items whose positions don't point to
            // an actual call in the source.
                if *src_id != contract_id =>
            {
                true
            }
            _ => false,
        });

        // We maintain a stack of active calls (whose head is being evaluated).  When encountering
        // an identifier (variable or record field), we see if it could serve as a function name
        // for the current active call. When a `Fun` is encountered, we check if this correspond to
        // the current active call, and if it does, the call description is moved to a stack of
        // processed calls.
        //
        // We also merge subcalls, in the sense that subcalls of larger calls are not considered
        // separately. `app1` is a subcall of `app2` if the position of `app1` is included in the
        // one of `app2` and the starting index is equal. We want `f a b c` to be reported as only
        // one big call to `f` rather than three nested calls `f a`, `f a b`, and `f a b c`.
        let mut pending: Vec<CallDescr> = Vec::new();
        let mut entered: Vec<CallDescr> = Vec::new();

        for elt in it {
            match elt {
                StackElem::Var { id, pos, .. }
                | StackElem::Field {
                    id,
                    pos_access: pos,
                    ..
                } => {
                    match pending.last_mut() {
                        Some(CallDescr {
                            head: ref mut head @ None,
                            span: span_call,
                        }) if pos.unwrap() <= *span_call => *head = Some(id.clone()),
                        _ => (),
                    };
                }
                StackElem::App(pos) => {
                    let span = pos.unwrap();
                    match pending.last() {
                        Some(CallDescr {
                            span: span_call, ..
                        }) if span <= *span_call && span.start == span_call.start => (),
                        _ => pending.push(CallDescr { head: None, span }),
                    }
                }
                StackElem::Fun(pos) => {
                    let span = pos.unwrap();
                    if pending
                        .last()
                        .map(|cdescr| cdescr.span == span)
                        .unwrap_or(false)
                    {
                        entered.push(pending.pop().unwrap());
                    }
                    // Otherwise, we are most probably entering a subcall () of the currently
                    // active call (e.g. in an multi-ary application `f g h`, a subcall would be `f
                    // g`). In any case, we do nothing.
                }
            }
        }

        entered.reverse();
        (entered, pending.pop())
    }

    /// Return the length of the callstack. Wrapper for `callstack.0.len()`.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Truncate the callstack at a certain size. Used e.g. to quickly drop the elements introduced
    /// during the strict evaluation of the operand of a primitive operator. Wrapper for
    /// `callstack.0.truncate(len)`.
    pub fn truncate(&mut self, len: usize) {
        self.0.truncate(len)
    }
}

impl From<CallStack> for Vec<StackElem> {
    fn from(cs: CallStack) -> Self {
        cs.0
    }
}
