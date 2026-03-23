//! Cooperative multitasking — multiple tasks with independent stacks,
//! event bindings, and round-robin scheduling.

use crate::platform::Platform;
use crate::stack::Stack;
use crate::value::Value;
use crate::vm::{StepResult, Vm};
use crate::{VmAction, VmError};

/// Task state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    /// Ready to run.
    Ready,
    /// Currently executing.
    Running,
    /// Yielded — will be resumed next round.
    Yielded,
    /// Yielded forever — only woken by events.
    Suspended,
    /// Finished (halted or error).
    Done,
}

/// A lightweight task descriptor.
///
/// Each task has its own data/return stacks, instruction pointer,
/// variables, and EVERY/TIMES state. Shares program memory and
/// string pool with the VM.
pub struct Task<const DS: usize = 64, const RS: usize = 32> {
    pub name: u16, // string pool index
    pub ds: Stack<DS>,
    pub rs: Stack<RS>,
    pub ip: usize,
    pub vars: [Value; 64],
    pub state: TaskState,
    /// TIMES loop counter stack.
    pub times_stack: [u32; 8],
    pub times_sp: usize,
    /// EVERY timing state.
    pub every_last: [u32; 8],
    pub every_sp: usize,
}

impl<const DS: usize, const RS: usize> Task<DS, RS> {
    pub fn new(name: u16, entry: usize) -> Self {
        Self {
            name,
            ds: Stack::new(),
            rs: Stack::new(),
            ip: entry,
            vars: [Value::I(0); 64],
            state: TaskState::Ready,
            times_stack: [0; 8],
            times_sp: 0,
            every_last: [0; 8],
            every_sp: 0,
        }
    }
}

/// An event binding: event_id → (task_index, word_offset).
#[derive(Clone, Copy)]
struct EventBinding {
    event_id: u16,
    task_idx: usize,
    word_offset: u16,
}

/// Round-robin cooperative scheduler with event dispatch.
///
/// Holds up to `N` tasks and `E` event bindings.
pub struct Scheduler<
    P: Platform,
    const N: usize = 8,
    const DS: usize = 64,
    const RS: usize = 32,
    const E: usize = 32,
> {
    pub tasks: [Option<Task<DS, RS>>; N],
    pub task_count: usize,
    /// Event binding table.
    bindings: [Option<EventBinding>; E],
    binding_count: usize,
    /// Pending events to dispatch (from EMIT or platform).
    event_queue: [u16; 16],
    event_queue_len: usize,
    _phantom: core::marker::PhantomData<P>,
}

impl<P: Platform, const N: usize, const DS: usize, const RS: usize, const E: usize>
    Scheduler<P, N, DS, RS, E>
{
    pub fn new() -> Self {
        Self {
            tasks: core::array::from_fn(|_| None),
            task_count: 0,
            bindings: [None; E],
            binding_count: 0,
            event_queue: [0; 16],
            event_queue_len: 0,
            _phantom: core::marker::PhantomData,
        }
    }

    /// Add a task. Returns its index.
    pub fn add_task(&mut self, task: Task<DS, RS>) -> Result<usize, VmError> {
        for i in 0..N {
            if self.tasks[i].is_none() {
                self.tasks[i] = Some(task);
                self.task_count += 1;
                return Ok(i);
            }
        }
        Err(VmError::TooManyTasks)
    }

    /// Find a task by name (string pool index). Returns its slot index.
    fn find_task(&self, name: u16) -> Option<usize> {
        for i in 0..N {
            if let Some(ref t) = self.tasks[i] {
                if t.name == name {
                    return Some(i);
                }
            }
        }
        None
    }

    /// Enqueue an event from outside (platform interrupt, etc.).
    pub fn emit_event(&mut self, event_id: u16) {
        if self.event_queue_len < self.event_queue.len() {
            self.event_queue[self.event_queue_len] = event_id;
            self.event_queue_len += 1;
        }
    }

    /// Register an event binding for a specific task.
    pub fn bind_event(
        &mut self,
        task_idx: usize,
        event_id: u16,
        word_offset: u16,
    ) -> Result<(), VmError> {
        if self.binding_count >= E {
            return Err(VmError::TooManyEvents);
        }
        self.bindings[self.binding_count] = Some(EventBinding {
            event_id,
            task_idx,
            word_offset,
        });
        self.binding_count += 1;
        Ok(())
    }

    /// Dispatch pending events: wake suspended tasks that have matching bindings.
    /// Only wakes `Suspended` tasks — `Yielded` tasks are already scheduled for
    /// the next round-robin pass and should not have their IP overwritten.
    fn dispatch_events(&mut self) -> Result<(), VmError> {
        for qi in 0..self.event_queue_len {
            let eid = self.event_queue[qi];
            for bi in 0..self.binding_count {
                if let Some(ref binding) = self.bindings[bi] {
                    if binding.event_id == eid {
                        let tidx = binding.task_idx;
                        if let Some(ref mut task) = self.tasks[tidx] {
                            if task.state == TaskState::Suspended {
                                // Wake the task and set IP to the handler word
                                task.rs.push(Value::I(task.ip as i32))?;
                                task.ip = binding.word_offset as usize;
                                task.state = TaskState::Ready;
                            }
                        }
                    }
                }
            }
        }
        self.event_queue_len = 0;
        Ok(())
    }

    /// Process VM actions generated during a task's execution.
    fn process_actions<
        const DN: usize,
        const STR_BYTES: usize,
        const STR_COUNT: usize,
        const BUF_BYTES: usize,
        const BUF_COUNT: usize,
    >(
        &mut self,
        current_task_idx: usize,
        vm: &mut Vm<P, DS, RS, STR_BYTES, STR_COUNT, BUF_BYTES, BUF_COUNT>,
        dict: &crate::dict::Dict<DN>,
    ) -> Result<(), VmError> {
        for action in vm.drain_actions() {
            match action {
                VmAction::StartTask(name_idx) => {
                    // Look up the task by name in dict to get entry point
                    if let Some(offset) = dict.lookup(name_idx) {
                        // Check if task already exists
                        if let Some(idx) = self.find_task(name_idx) {
                            if let Some(ref mut t) = self.tasks[idx] {
                                if t.state == TaskState::Done {
                                    // Restart it — fully reset state
                                    t.ip = offset as usize;
                                    t.state = TaskState::Ready;
                                    t.ds.clear();
                                    t.rs.clear();
                                    t.vars = [Value::I(0); 64];
                                    t.times_sp = 0;
                                    t.every_sp = 0;
                                    t.every_last = [0; 8];
                                }
                                // Already running — ignore
                            }
                        } else {
                            // Create new task
                            let task = Task::new(name_idx, offset as usize);
                            self.add_task(task)?;
                        }
                    }
                }
                VmAction::StopTask(name_idx) => {
                    if let Some(idx) = self.find_task(name_idx) {
                        if let Some(ref mut t) = self.tasks[idx] {
                            t.state = TaskState::Done;
                        }
                    }
                }
                VmAction::EmitEvent(event_id) => {
                    self.emit_event(event_id);
                }
                VmAction::BindEvent {
                    event_id,
                    word_offset,
                } => {
                    self.bind_event(current_task_idx, event_id, word_offset)?;
                }
            }
        }
        Ok(())
    }

    /// Run one round-robin tick. Each ready task gets up to `max_steps` instructions.
    ///
    /// `dict` is needed to resolve task names for START commands.
    pub fn tick<
        const DN: usize,
        const STR_BYTES: usize,
        const STR_COUNT: usize,
        const BUF_BYTES: usize,
        const BUF_COUNT: usize,
    >(
        &mut self,
        vm: &mut Vm<P, DS, RS, STR_BYTES, STR_COUNT, BUF_BYTES, BUF_COUNT>,
        dict: &crate::dict::Dict<DN>,
        max_steps: u32,
    ) -> Result<bool, VmError> {
        // First, dispatch any pending events
        self.dispatch_events()?;

        let mut any_active = false;

        for i in 0..N {
            let task = match &mut self.tasks[i] {
                Some(t) if t.state == TaskState::Ready || t.state == TaskState::Yielded => t,
                _ => continue,
            };

            any_active = true;
            task.state = TaskState::Running;

            // Save VM state, load task state
            let saved_ip = vm.ip;
            let saved_vars = vm.vars;
            let saved_times_stack = vm.times_stack;
            let saved_times_sp = vm.times_sp;
            let saved_every_last = vm.every_last;
            let saved_every_sp = vm.every_sp;
            let saved_halted = vm.halted;

            vm.ip = task.ip;
            vm.vars = task.vars;
            vm.times_stack = task.times_stack;
            vm.times_sp = task.times_sp;
            vm.every_last = task.every_last;
            vm.every_sp = task.every_sp;
            vm.halted = false; // Reset so this task can execute
            core::mem::swap(&mut vm.ds, &mut task.ds);
            core::mem::swap(&mut vm.rs, &mut task.rs);

            let result = vm.run_steps(max_steps);

            // Save task state back
            task.ip = vm.ip;
            task.vars = vm.vars;
            task.times_stack = vm.times_stack;
            task.times_sp = vm.times_sp;
            task.every_last = vm.every_last;
            task.every_sp = vm.every_sp;
            core::mem::swap(&mut vm.ds, &mut task.ds);
            core::mem::swap(&mut vm.rs, &mut task.rs);

            vm.ip = saved_ip;
            vm.vars = saved_vars;
            vm.times_stack = saved_times_stack;
            vm.times_sp = saved_times_sp;
            vm.every_last = saved_every_last;
            vm.every_sp = saved_every_sp;
            vm.halted = saved_halted;

            match result {
                StepResult::Continue | StepResult::Yielded => {
                    task.state = TaskState::Yielded;
                }
                StepResult::YieldedForever => {
                    task.state = TaskState::Suspended;
                }
                StepResult::Halted | StepResult::Error(_) => {
                    task.state = TaskState::Done;
                }
            }

            // Process any actions the task generated
            self.process_actions(i, vm, dict)?;
        }

        Ok(any_active)
    }

    /// Check if any tasks are still active.
    pub fn any_active(&self) -> bool {
        self.tasks.iter().any(|t| {
            matches!(
                t,
                Some(task) if task.state == TaskState::Ready
                    || task.state == TaskState::Yielded
                    || task.state == TaskState::Running
                    || task.state == TaskState::Suspended
            )
        })
    }
}
