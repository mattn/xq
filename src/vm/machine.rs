use crate::{
    data_structure::{PStack, PVector},
    vm::{
        bytecode::{Closure, NamedFunction},
        error::QueryExecutionError,
        intrinsic::truthy,
        Address, ByteCode, Program, Result, ScopeId, ScopedSlot, Value,
    },
};
use thiserror::Error;

#[derive(Debug, Clone, Eq, PartialEq, Error)]
pub(crate) enum ProgramError {
    #[error("tried to pop an empty stack")]
    PopEmptyStack,
    #[error("tried to use an uninitialized scope")]
    UninitializedScope,
    #[error("tried to load from an uninitialized slot of a scope")]
    UnknownSlot,
    #[error("tried to load from an uninitialized slot of a scope")]
    UninitializedSlot,
    #[error("tried to pop a scope but there was no scope to pop")]
    PopEmptyScope,
    #[error("tried to restore an unknown scope")]
    PopUnknownScope,
}

#[derive(Debug, Clone)]
struct Scope {
    slots: PVector<Option<Value>>,
    closure_slots: PVector<Option<Closure>>,
}

impl Scope {
    fn new(variable_cnt: usize, closure_cnt: usize) -> Self {
        Self {
            slots: std::iter::repeat(None).take(variable_cnt).collect(),
            closure_slots: std::iter::repeat(None).take(closure_cnt).collect(),
        }
    }
}

#[derive(Debug)]
struct Machine {
    env: Environment,
}

#[derive(Debug)]
struct Environment {
    program: Program,
    forks: Vec<State>,
}

#[derive(Debug, Clone)]
struct State {
    pc: Address,
    stack: PStack<Value>,

    scopes: PVector<(Option<Scope>, PStack<Scope>)>,
    scope_stack: PStack<(Address, ScopeId)>, // (pc, pushed_scope)
    closure_stack: PStack<Closure>,
}

impl Environment {
    fn push_fork(&mut self, state: &State, new_pc: Address) {
        let mut new_state = state.clone();
        new_state.pc = new_pc;
        self.forks.push(new_state);
    }

    fn pop_fork(&mut self) -> Option<State> {
        self.forks.pop()
    }
}

impl State {
    fn new(pc: Address) -> Self {
        State {
            pc,
            stack: Default::default(),
            scopes: Default::default(),
            scope_stack: Default::default(),
            closure_stack: Default::default(),
        }
    }

    fn push(&mut self, item: Value) {
        self.stack.push(item)
    }

    fn pop(&mut self) -> Value {
        self.stack.pop().ok_or(ProgramError::PopEmptyStack).unwrap()
    }

    fn dup(&mut self) {
        let value = self.pop();
        self.push(value.clone());
        self.push(value);
    }

    fn push_closure(&mut self, closure: Closure) {
        self.closure_stack.push(closure)
    }

    fn pop_closure(&mut self) -> Closure {
        self.closure_stack
            .pop()
            .ok_or(ProgramError::PopEmptyStack)
            .unwrap()
    }

    fn slot(&mut self, scoped_slot: &ScopedSlot) -> &mut Option<Value> {
        let scope = self
            .scopes
            .get_mut(scoped_slot.0 .0)
            .and_then(|(x, _)| x.as_mut())
            .ok_or(ProgramError::UninitializedScope)
            .unwrap();
        scope
            .slots
            .get_mut(scoped_slot.1)
            .ok_or(ProgramError::UnknownSlot)
            .unwrap()
    }

    fn closure_slot(&mut self, scoped_slot: &ScopedSlot) -> &mut Option<Closure> {
        let scope = self
            .scopes
            .get_mut(scoped_slot.0 .0)
            .and_then(|(x, _)| x.as_mut())
            .ok_or(ProgramError::UninitializedScope)
            .unwrap();
        scope
            .closure_slots
            .get_mut(scoped_slot.1)
            .ok_or(ProgramError::UnknownSlot)
            .unwrap()
    }

    fn push_scope(
        &mut self,
        scope_id: ScopeId,
        variable_cnt: usize,
        closure_cnt: usize,
        pc: Address,
    ) {
        if self.scopes.len() <= scope_id.0 {
            self.scopes.extend(
                std::iter::repeat_with(|| (None, PStack::new()))
                    .take(scope_id.0 - self.scopes.len() + 1),
            )
        }
        let (scope, stack) = &mut self.scopes[scope_id.0];
        if let Some(prev_scope) = scope.replace(Scope::new(variable_cnt, closure_cnt)) {
            stack.push(prev_scope);
        }
        self.scope_stack.push((pc, scope_id));
    }

    fn pop_scope(&mut self) -> Address {
        let (pc, scope_id) = self
            .scope_stack
            .pop()
            .ok_or(ProgramError::PopEmptyScope)
            .unwrap();
        let (current, prev) = self
            .scopes
            .get_mut(scope_id.0)
            .ok_or(ProgramError::PopUnknownScope)
            .unwrap();
        assert!(current.is_some(), "Pop unknown scope");
        *current = prev.pop();
        pc
    }
}

impl Machine {
    fn new(program: Program) -> Self {
        let state = State::new(program.entry_point);
        Self {
            env: Environment {
                program,
                forks: vec![state],
            },
        }
    }
}

impl Iterator for Machine {
    type Item = Result<Value>;

    fn next(&mut self) -> Option<Self::Item> {
        run_code(&mut self.env)
    }
}

fn run_code(env: &mut Environment) -> Option<Result<Value>> {
    let mut err: Option<QueryExecutionError> = None;
    let mut call_pc: Option<Address> = None;
    let mut state = env.pop_fork()?;
    while let Some(code) = env.program.fetch_code(state.pc) {
        use ByteCode::*;
        match code {
            Unreachable => panic!("Reached to the unreachable"),
            PlaceHolder => panic!("Reached to a place holder"),
            Nop => {}
            Push(v) => {
                state.push(v.clone());
            }
            Pop => {
                state.pop();
            }
            Dup => {
                state.dup();
            }
            Const(v) => {
                state.pop();
                state.push(v.clone())
            }
            Load(scoped_slot) => {
                let value = state
                    .slot(scoped_slot)
                    .as_ref()
                    .ok_or(ProgramError::UninitializedSlot)
                    .unwrap()
                    .clone();
                state.push(value);
            }
            Store(scoped_slot) => {
                let value = state.pop();
                state.slot(scoped_slot).replace(value);
            }
            PushClosure(closure) => {
                state.push_closure(*closure);
            }
            StoreClosure(slot) => {
                let closure = state.pop_closure();
                state.closure_slot(slot).replace(closure);
            }
            Object => todo!("Implement {:?}", code),
            Append(scoped_slot) => {
                let value = state.pop();
                let slot_item = state
                    .slot(scoped_slot)
                    .as_mut()
                    .ok_or(ProgramError::UninitializedSlot)
                    .unwrap();
                match slot_item {
                    Value::Array(v) => {
                        v.push_back(value);
                    }
                    _ => {
                        panic!("expected a array to append to, but was not an array");
                    }
                }
            }
            Fork { fork_pc } => {
                let fork_pc = *fork_pc;
                env.push_fork(&state, fork_pc);
            }
            ForkTryBegin => todo!("Implement {:?}", code),
            ForkTryEnd => todo!("Implement {:?}", code),
            ForkAlt => todo!("Implement {:?}", code),
            ForkLabel => todo!("Implement {:?}", code),
            Backtrack => todo!("Implement {:?}", code),
            Jump(address) => {
                state.pc = *address;
                continue;
            }
            JumpUnless(address) => {
                let value = state.pop();
                if !truthy(value) {
                    state.pc = *address;
                    continue;
                }
            }
            CallClosure {
                slot,
                return_address,
            } => {
                let closure = state
                    .closure_slot(slot)
                    .ok_or(ProgramError::UninitializedSlot)
                    .unwrap();
                assert_eq!(call_pc.replace(*return_address), None);
                state.pc = closure.0;
                continue;
            }
            Call {
                function,
                return_address,
            } => {
                assert_eq!(call_pc.replace(*return_address), None);
                state.pc = *function;
                continue;
            }
            PushPC => todo!("Implement {:?}", code),
            CallPC => todo!("Implement {:?}", code),
            NewScope {
                id,
                variable_cnt,
                closure_cnt,
            } => {
                let return_address = call_pc
                    .take()
                    .expect("NewScope should be called after Call");
                state.push_scope(*id, *variable_cnt, *closure_cnt, return_address);
            }
            Ret => {
                let return_address = state.pop_scope();
                state.pc = return_address;
                continue;
            }
            Output => {
                return if let Some(err) = err {
                    Some(Err(err))
                } else {
                    let value = state.pop();
                    Some(Ok(value))
                }
            }
            Each => todo!("Implement {:?}", code),
            ExpBegin => todo!("Implement {:?}", code),
            ExpEnd => todo!("Implement {:?}", code),
            PathBegin => todo!("Implement {:?}", code),
            PathEnd => todo!("Implement {:?}", code),
            Intrinsic1(NamedFunction { name: _name, func }) => {
                let arg = state.pop();
                match func(arg) {
                    Ok(value) => state.push(value),
                    Err(e) => err = Some(e),
                }
            }
            Intrinsic2(NamedFunction { name: _name, func }) => {
                let rhs = state.pop();
                let lhs = state.pop();
                match func(lhs, rhs) {
                    Ok(value) => state.push(value),
                    Err(e) => err = Some(e),
                }
            }
        }
        state.pc.next();
    }
    None
}
