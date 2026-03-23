use spore_core::*;

/// Null platform for testing — all ops return PlatformError.
struct NullPlatform;

impl Platform for NullPlatform {}

/// Mock platform that tracks log messages and provides millis.
struct MockPlatform {
    logs: Vec<String>,
    millis: u32,
}

impl MockPlatform {
    fn new() -> Self {
        Self {
            logs: Vec::new(),
            millis: 0,
        }
    }
}

impl Platform for MockPlatform {
    fn log(&mut self, msg: &str) -> Result<(), VmError> {
        self.logs.push(msg.to_string());
        Ok(())
    }

    fn millis(&self) -> Result<u32, VmError> {
        Ok(self.millis)
    }

    fn heap_free(&self) -> Result<u32, VmError> {
        Ok(512_000)
    }

    fn delay_ms(&mut self, ms: u32) -> Result<(), VmError> {
        self.millis += ms;
        Ok(())
    }

    fn gpio_mode(&mut self, _pin: i32, _mode: i32) -> Result<(), VmError> {
        Ok(())
    }

    fn gpio_write(&mut self, _pin: i32, _val: i32) -> Result<(), VmError> {
        Ok(())
    }

    fn gpio_read(&mut self, _pin: i32) -> Result<i32, VmError> {
        Ok(0)
    }
}

fn run_ops(ops: &[Op]) -> Vm<NullPlatform> {
    let mut vm = Vm::<NullPlatform>::new(NullPlatform);
    vm.load(ops);
    vm.run();
    vm
}

// ---- Basic arithmetic ----

#[test]
fn test_add_integers() {
    let vm = run_ops(&[Op::Lit(3), Op::Lit(4), Op::Add, Op::Halt]);
    assert_eq!(vm.ds.peek().unwrap().as_int(), 7);
}

#[test]
fn test_add_floats() {
    let vm = run_ops(&[Op::FLit(1.5), Op::FLit(2.5), Op::Add, Op::Halt]);
    assert_eq!(vm.ds.peek().unwrap().as_float(), 4.0);
}

#[test]
fn test_type_promotion() {
    let vm = run_ops(&[Op::Lit(2), Op::FLit(3.5), Op::Add, Op::Halt]);
    assert_eq!(vm.ds.peek().unwrap().as_float(), 5.5);
}

#[test]
fn test_sub() {
    let vm = run_ops(&[Op::Lit(10), Op::Lit(3), Op::Sub, Op::Halt]);
    assert_eq!(vm.ds.peek().unwrap().as_int(), 7);
}

#[test]
fn test_mul() {
    let vm = run_ops(&[Op::Lit(6), Op::Lit(7), Op::Mul, Op::Halt]);
    assert_eq!(vm.ds.peek().unwrap().as_int(), 42);
}

#[test]
fn test_div() {
    let vm = run_ops(&[Op::Lit(15), Op::Lit(4), Op::Div, Op::Halt]);
    assert_eq!(vm.ds.peek().unwrap().as_int(), 3);
}

#[test]
fn test_div_by_zero() {
    let mut vm = Vm::<NullPlatform>::new(NullPlatform);
    vm.load(&[Op::Lit(10), Op::Lit(0), Op::Div]);
    let result = vm.run();
    assert_eq!(result, StepResult::Error(VmError::DivisionByZero));
}

#[test]
fn test_mod() {
    let vm = run_ops(&[Op::Lit(17), Op::Lit(5), Op::Mod, Op::Halt]);
    assert_eq!(vm.ds.peek().unwrap().as_int(), 2);
}

#[test]
fn test_neg() {
    let vm = run_ops(&[Op::Lit(42), Op::Neg, Op::Halt]);
    assert_eq!(vm.ds.peek().unwrap().as_int(), -42);
}

#[test]
fn test_abs() {
    let vm = run_ops(&[Op::Lit(-42), Op::Abs, Op::Halt]);
    assert_eq!(vm.ds.peek().unwrap().as_int(), 42);
}

#[test]
fn test_min_max() {
    let vm = run_ops(&[Op::Lit(3), Op::Lit(7), Op::Min, Op::Halt]);
    assert_eq!(vm.ds.peek().unwrap().as_int(), 3);

    let vm = run_ops(&[Op::Lit(3), Op::Lit(7), Op::Max, Op::Halt]);
    assert_eq!(vm.ds.peek().unwrap().as_int(), 7);
}

// ---- Stack manipulation ----

#[test]
fn test_dup() {
    let vm = run_ops(&[Op::Lit(5), Op::Dup, Op::Halt]);
    assert_eq!(vm.ds.depth(), 2);
}

#[test]
fn test_swap() {
    let vm = run_ops(&[Op::Lit(1), Op::Lit(2), Op::Swap, Op::Halt]);
    assert_eq!(vm.ds.peek_at(0).unwrap().as_int(), 1);
    assert_eq!(vm.ds.peek_at(1).unwrap().as_int(), 2);
}

#[test]
fn test_over() {
    let vm = run_ops(&[Op::Lit(1), Op::Lit(2), Op::Over, Op::Halt]);
    assert_eq!(vm.ds.depth(), 3);
    assert_eq!(vm.ds.peek_at(0).unwrap().as_int(), 1);
}

#[test]
fn test_rot() {
    let vm = run_ops(&[Op::Lit(1), Op::Lit(2), Op::Lit(3), Op::Rot, Op::Halt]);
    assert_eq!(vm.ds.peek_at(0).unwrap().as_int(), 1);
    assert_eq!(vm.ds.peek_at(1).unwrap().as_int(), 3);
    assert_eq!(vm.ds.peek_at(2).unwrap().as_int(), 2);
}

#[test]
fn test_depth() {
    let vm = run_ops(&[Op::Lit(1), Op::Lit(2), Op::Lit(3), Op::Depth, Op::Halt]);
    assert_eq!(vm.ds.peek().unwrap().as_int(), 3);
}

// ---- Comparison ----

#[test]
fn test_comparisons() {
    let vm = run_ops(&[Op::Lit(3), Op::Lit(5), Op::Lt, Op::Halt]);
    assert!(vm.ds.peek().unwrap().as_bool());

    let vm = run_ops(&[Op::Lit(5), Op::Lit(3), Op::Gt, Op::Halt]);
    assert!(vm.ds.peek().unwrap().as_bool());

    let vm = run_ops(&[Op::Lit(3), Op::Lit(3), Op::Eq, Op::Halt]);
    assert!(vm.ds.peek().unwrap().as_bool());

    let vm = run_ops(&[Op::Lit(3), Op::Lit(4), Op::Neq, Op::Halt]);
    assert!(vm.ds.peek().unwrap().as_bool());
}

// ---- Logic ----

#[test]
fn test_logic() {
    let vm = run_ops(&[Op::BLit(true), Op::BLit(false), Op::And, Op::Halt]);
    assert!(!vm.ds.peek().unwrap().as_bool());

    let vm = run_ops(&[Op::BLit(true), Op::BLit(false), Op::Or, Op::Halt]);
    assert!(vm.ds.peek().unwrap().as_bool());

    let vm = run_ops(&[Op::BLit(true), Op::Not, Op::Halt]);
    assert!(!vm.ds.peek().unwrap().as_bool());
}

#[test]
fn test_bitwise() {
    let vm = run_ops(&[Op::Lit(0xFF), Op::Lit(0x0F), Op::And, Op::Halt]);
    assert_eq!(vm.ds.peek().unwrap().as_int(), 0x0F);

    let vm = run_ops(&[Op::Lit(1), Op::Lit(4), Op::Shl, Op::Halt]);
    assert_eq!(vm.ds.peek().unwrap().as_int(), 16);
}

// ---- Control flow ----

#[test]
fn test_if_true() {
    let vm = run_ops(&[
        Op::BLit(true),
        Op::If(4),
        Op::Lit(42),
        Op::Else(5),
        Op::Then, // index 4
        Op::Halt, // index 5
    ]);
    assert_eq!(vm.ds.peek().unwrap().as_int(), 42);
}

#[test]
fn test_if_false() {
    let vm = run_ops(&[
        Op::BLit(false),
        Op::If(3), // false → jump to Then at index 3
        Op::Lit(42),
        Op::Then,
        Op::Halt,
    ]);
    assert_eq!(vm.ds.depth(), 0);
}

#[test]
fn test_if_else() {
    let vm = run_ops(&[
        Op::BLit(false),
        Op::If(4),      // false → jump to 4
        Op::Lit(42),
        Op::Else(5),    // jump to 5 (Then)
        Op::Lit(99),    // index 4: else branch
        Op::Then,       // index 5
        Op::Halt,
    ]);
    assert_eq!(vm.ds.peek().unwrap().as_int(), 99);
}

#[test]
fn test_times_loop() {
    let vm = run_ops(&[
        Op::Lit(0),      // accumulator
        Op::Lit(5),      // count
        Op::Times(6),    // pop 5, loop body at 3..5
        Op::Lit(1),
        Op::Add,
        Op::EndTimes(3), // decrement, if >0 jump to 3
        Op::Halt,
    ]);
    assert_eq!(vm.ds.peek().unwrap().as_int(), 5);
}

#[test]
fn test_begin_until() {
    let vm = run_ops(&[
        Op::Lit(0),       // counter
        Op::Begin,        // index 1
        Op::Lit(1),
        Op::Add,
        Op::Dup,
        Op::Lit(5),
        Op::Eq,
        Op::Until(1),     // if false, jump back to Begin at index 1
        Op::Halt,
    ]);
    assert_eq!(vm.ds.peek().unwrap().as_int(), 5);
}

#[test]
fn test_loop_break() {
    // LOOP: increment counter, BREAK when == 3
    // Loop stores offset to AFTER EndLoop (for Break to use)
    // EndLoop stores offset to Loop itself (to re-enter)
    let vm = run_ops(&[
        Op::Lit(0),        // 0: counter
        Op::Loop(10),      // 1: loop start; Break exit → 10
        Op::Lit(1),        // 2
        Op::Add,           // 3
        Op::Dup,           // 4
        Op::Lit(3),        // 5
        Op::Eq,            // 6
        Op::If(9),         // 7: if false → jump to 9 (EndLoop); true → fall through
        Op::Break(1),      // 8: exit loop → reads Loop(10) at index 1, jumps to 10
        Op::EndLoop(1),    // 9: jump back to Loop at 1
        Op::Halt,          // 10
    ]);
    assert_eq!(vm.ds.peek().unwrap().as_int(), 3);
}

// ---- Variables ----

#[test]
fn test_variables() {
    let vm = run_ops(&[
        Op::Lit(42),
        Op::Store(0),
        Op::Lit(99),
        Op::Store(1),
        Op::Fetch(0),
        Op::Fetch(1),
        Op::Add,
        Op::Halt,
    ]);
    assert_eq!(vm.ds.peek().unwrap().as_int(), 141);
}

// ---- Words (Call/Return) ----

#[test]
fn test_call_return() {
    let mut vm = Vm::<NullPlatform>::new(NullPlatform);
    vm.load(&[
        Op::Dup,       // 0: double body
        Op::Add,       // 1
        Op::Return,    // 2
        Op::Lit(21),   // 3: main
        Op::Call(0),   // 4
        Op::Halt,      // 5
    ]);
    vm.ip = 3;
    vm.run();
    assert_eq!(vm.ds.peek().unwrap().as_int(), 42);
}

// ---- Type conversion ----

#[test]
fn test_type_conversion() {
    let vm = run_ops(&[Op::Lit(42), Op::ItoF, Op::Halt]);
    assert_eq!(vm.ds.peek().unwrap().as_float(), 42.0);

    let vm = run_ops(&[Op::FLit(3.7), Op::FtoI, Op::Halt]);
    assert_eq!(vm.ds.peek().unwrap().as_int(), 3);
}

// ---- Parser ----

#[test]
fn test_parse_simple() {
    let mut strings = StringPool::<2048, 128>::new();
    let mut dict = Dict::<64>::new();
    let result = parse("LIT 3 LIT 4 ADD HALT", &mut strings, &mut dict).unwrap();
    assert_eq!(result.len, 4);

    let mut vm = Vm::<NullPlatform>::new(NullPlatform);
    vm.strings = strings;
    vm.load(&result.ops[..result.len]);
    vm.program_len = result.len;
    vm.run();
    assert_eq!(vm.ds.peek().unwrap().as_int(), 7);
}

#[test]
fn test_parse_def_and_call() {
    let mut strings = StringPool::<2048, 128>::new();
    let mut dict = Dict::<64>::new();
    let input = r#"
        DEF double
          DUP ADD
        END
        LIT 21 double HALT
    "#;
    let result = parse(input, &mut strings, &mut dict).unwrap();

    let mut vm = Vm::<NullPlatform>::new(NullPlatform);
    vm.strings = strings;
    vm.load(&result.ops[..result.len]);
    vm.program_len = result.len;
    vm.ip = 3; // past DEF...END (DUP ADD Return = 3 ops)
    vm.run();
    assert_eq!(vm.ds.peek().unwrap().as_int(), 42);
}

#[test]
fn test_parse_if_else() {
    let mut strings = StringPool::<2048, 128>::new();
    let mut dict = Dict::<64>::new();
    let input = "FALSE IF LIT 1 ELSE LIT 2 THEN HALT";
    let result = parse(input, &mut strings, &mut dict).unwrap();

    let mut vm = Vm::<NullPlatform>::new(NullPlatform);
    vm.load(&result.ops[..result.len]);
    vm.program_len = result.len;
    vm.run();
    assert_eq!(vm.ds.peek().unwrap().as_int(), 2);
}

#[test]
fn test_parse_string_literal() {
    let mut strings = StringPool::<2048, 128>::new();
    let mut dict = Dict::<64>::new();
    let input = r#"STR "hello world" HALT"#;
    let result = parse(input, &mut strings, &mut dict).unwrap();

    let mut vm = Vm::<NullPlatform>::new(NullPlatform);
    vm.strings = strings;
    vm.load(&result.ops[..result.len]);
    vm.program_len = result.len;
    vm.run();

    let idx = vm.ds.peek().unwrap().as_str_index().unwrap();
    assert_eq!(vm.strings.get(idx).unwrap(), "hello world");
}

#[test]
fn test_parse_float_literal() {
    let mut strings = StringPool::<2048, 128>::new();
    let mut dict = Dict::<64>::new();
    let input = "FLIT 3.14 HALT";
    let result = parse(input, &mut strings, &mut dict).unwrap();

    let mut vm = Vm::<NullPlatform>::new(NullPlatform);
    vm.load(&result.ops[..result.len]);
    vm.program_len = result.len;
    vm.run();

    let f = vm.ds.peek().unwrap().as_float();
    assert!((f - 3.14).abs() < 0.01);
}

#[test]
fn test_parse_hex_literal() {
    let mut strings = StringPool::<2048, 128>::new();
    let mut dict = Dict::<64>::new();
    let input = "LIT 0x76 HALT";
    let result = parse(input, &mut strings, &mut dict).unwrap();

    let mut vm = Vm::<NullPlatform>::new(NullPlatform);
    vm.load(&result.ops[..result.len]);
    vm.program_len = result.len;
    vm.run();
    assert_eq!(vm.ds.peek().unwrap().as_int(), 0x76);
}

#[test]
fn test_parse_comments() {
    let mut strings = StringPool::<2048, 128>::new();
    let mut dict = Dict::<64>::new();
    let input = "LIT 1 \\ this is a comment\nLIT 2 ADD HALT";
    let result = parse(input, &mut strings, &mut dict).unwrap();

    let mut vm = Vm::<NullPlatform>::new(NullPlatform);
    vm.load(&result.ops[..result.len]);
    vm.program_len = result.len;
    vm.run();
    assert_eq!(vm.ds.peek().unwrap().as_int(), 3);
}

#[test]
fn test_parse_unclosed_def() {
    let mut strings = StringPool::<2048, 128>::new();
    let mut dict = Dict::<64>::new();
    let result = parse("DEF foo LIT 1", &mut strings, &mut dict);
    assert_eq!(result.unwrap_err(), VmError::ParseError);
}

#[test]
fn test_parse_unclosed_task() {
    let mut strings = StringPool::<2048, 128>::new();
    let mut dict = Dict::<64>::new();
    let result = parse("TASK main LIT 1", &mut strings, &mut dict);
    assert_eq!(result.unwrap_err(), VmError::ParseError);
}

#[test]
fn test_parse_start_stop() {
    let mut strings = StringPool::<2048, 128>::new();
    let mut dict = Dict::<64>::new();
    let input = r#"
        TASK worker
          YIELD
        ENDTASK
        TASK main
          START worker
        ENDTASK
    "#;
    let result = parse(input, &mut strings, &mut dict).unwrap();
    // Check START emitted an Op::Start
    let mut found_start = false;
    for i in 0..result.len {
        if let Op::Start(_) = result.ops[i] {
            found_start = true;
            break;
        }
    }
    assert!(found_start, "START should emit Op::Start");
}

#[test]
fn test_parse_variables() {
    let mut strings = StringPool::<2048, 128>::new();
    let mut dict = Dict::<64>::new();
    let input = "VAR x LIT 42 STORE x FETCH x HALT";
    let result = parse(input, &mut strings, &mut dict).unwrap();

    let mut vm = Vm::<NullPlatform>::new(NullPlatform);
    vm.strings = strings;
    vm.load(&result.ops[..result.len]);
    vm.program_len = result.len;
    vm.run();
    assert_eq!(vm.ds.peek().unwrap().as_int(), 42);
}

// ---- Platform ops with mock ----

#[test]
fn test_platform_log() {
    let mut vm = Vm::<MockPlatform>::new(MockPlatform::new());
    let idx = vm.strings.intern("hello from spore").unwrap();
    vm.load(&[Op::SLit(idx), Op::PLog, Op::Halt]);
    vm.run();
    assert_eq!(vm.platform.logs, vec!["hello from spore"]);
}

#[test]
fn test_platform_heap_free() {
    let mut vm = Vm::<MockPlatform>::new(MockPlatform::new());
    vm.load(&[Op::PHeapFree, Op::Halt]);
    vm.run();
    assert_eq!(vm.ds.peek().unwrap().as_int(), 512_000);
}

#[test]
fn test_platform_delay() {
    let mut vm = Vm::<MockPlatform>::new(MockPlatform::new());
    vm.load(&[Op::Lit(1000), Op::PDelayMs, Op::PMillis, Op::Halt]);
    vm.run();
    assert_eq!(vm.ds.peek().unwrap().as_int(), 1000);
}

// ---- Stack error handling ----

#[test]
fn test_stack_underflow() {
    let mut vm = Vm::<NullPlatform>::new(NullPlatform);
    vm.load(&[Op::Drop]);
    let result = vm.run();
    assert_eq!(result, StepResult::Error(VmError::StackUnderflow));
}

// ---- ItoStr / FtoStr ----

#[test]
fn test_int_to_str() {
    let mut vm = Vm::<NullPlatform>::new(NullPlatform);
    vm.load(&[Op::Lit(42), Op::ItoStr, Op::Halt]);
    vm.run();
    let idx = vm.ds.peek().unwrap().as_str_index().unwrap();
    assert_eq!(vm.strings.get(idx).unwrap(), "42");
}

#[test]
fn test_float_to_str() {
    let mut vm = Vm::<NullPlatform>::new(NullPlatform);
    vm.load(&[Op::FLit(3.14), Op::FtoStr, Op::Halt]);
    vm.run();
    let idx = vm.ds.peek().unwrap().as_str_index().unwrap();
    let s = vm.strings.get(idx).unwrap();
    assert!(s.starts_with("3.1"), "got: {}", s);
}

// ---- Buffer pool ----

#[test]
fn test_buffer_pool() {
    let mut pool = BufferPool::<256, 8>::new();
    let idx = pool.alloc_from(&[1, 2, 3, 4]).unwrap();
    assert_eq!(pool.get(idx).unwrap(), &[1, 2, 3, 4]);

    let idx2 = pool.alloc(3).unwrap();
    assert_eq!(pool.get(idx2).unwrap(), &[0, 0, 0]);

    // Mutate
    pool.get_mut(idx2).unwrap()[0] = 42;
    assert_eq!(pool.get(idx2).unwrap()[0], 42);
}

// ---- VM actions (START/STOP/events) ----

#[test]
fn test_vm_start_action() {
    let mut vm = Vm::<NullPlatform>::new(NullPlatform);
    let name_idx = vm.strings.intern("worker").unwrap();
    vm.load(&[Op::Start(name_idx), Op::Halt]);
    vm.run();

    let actions: Vec<_> = vm.drain_actions().collect();
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0], VmAction::StartTask(name_idx));
}

#[test]
fn test_vm_emit_event_action() {
    let mut vm = Vm::<NullPlatform>::new(NullPlatform);
    let evt_idx = vm.strings.intern("my_event").unwrap();
    vm.load(&[Op::EmitEvent(evt_idx), Op::Halt]);
    vm.run();

    let actions: Vec<_> = vm.drain_actions().collect();
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0], VmAction::EmitEvent(evt_idx));
}

#[test]
fn test_vm_bind_event_action() {
    let mut vm = Vm::<NullPlatform>::new(NullPlatform);
    let evt_idx = vm.strings.intern("GPIO_RISE:4").unwrap();
    vm.load(&[Op::On(evt_idx, 42), Op::Halt]);
    vm.run();

    let actions: Vec<_> = vm.drain_actions().collect();
    assert_eq!(actions.len(), 1);
    assert_eq!(
        actions[0],
        VmAction::BindEvent {
            event_id: evt_idx,
            word_offset: 42
        }
    );
}

// ---- Scheduler with multi-task ----

#[test]
fn test_scheduler_basic() {
    let mut vm = Vm::<MockPlatform>::new(MockPlatform::new());
    let mut dict = Dict::<64>::new();

    // Task A: push 10, halt
    let a_name = vm.strings.intern("task_a").unwrap();
    dict.define(a_name, 0).unwrap();
    vm.program[0] = Op::Lit(10);
    vm.program[1] = Op::Halt;
    vm.program_len = 2;

    let mut sched = Scheduler::<MockPlatform>::new();
    sched.add_task(Task::new(a_name, 0)).unwrap();

    let active = sched.tick(&mut vm, &dict, 100).unwrap();
    assert!(active);

    // Task should be Done now
    assert_eq!(sched.tasks[0].as_ref().unwrap().state, TaskState::Done);
    // It should have pushed 10 onto its own stack
    assert_eq!(
        sched.tasks[0].as_ref().unwrap().ds.peek().unwrap().as_int(),
        10
    );
}

#[test]
fn test_scheduler_event_wakeup() {
    let mut vm = Vm::<MockPlatform>::new(MockPlatform::new());
    let mut dict = Dict::<64>::new();

    // A handler word at offset 0: push 99, return
    vm.program[0] = Op::Lit(99);
    vm.program[1] = Op::Return;
    // Task body at offset 2: bind event, yield forever, halt
    let evt_name = vm.strings.intern("test_evt").unwrap();
    vm.program[2] = Op::On(evt_name, 0); // bind test_evt → handler at 0
    vm.program[3] = Op::YieldForever;
    vm.program[4] = Op::Halt;
    vm.program_len = 5;

    let task_name = vm.strings.intern("listener").unwrap();
    dict.define(task_name, 2).unwrap();

    let mut sched = Scheduler::<MockPlatform>::new();
    sched.add_task(Task::new(task_name, 2)).unwrap();

    // First tick: task runs On + YieldForever → Suspended
    sched.tick(&mut vm, &dict, 100).unwrap();
    assert_eq!(
        sched.tasks[0].as_ref().unwrap().state,
        TaskState::Suspended
    );

    // Process the BindEvent action manually (tick already did it)
    // Now emit the event
    sched.emit_event(evt_name);

    // Next tick: event wakes the task, handler pushes 99
    sched.tick(&mut vm, &dict, 100).unwrap();

    let task = sched.tasks[0].as_ref().unwrap();
    // Task should have 99 on its stack (from handler)
    assert_eq!(task.ds.peek().unwrap().as_int(), 99);
}

// ---- Parse the full cactus monitor example from the spec ----

#[test]
fn test_parse_cactus_monitor() {
    let mut strings = StringPool::<4096, 256>::new();
    let mut dict = Dict::<64>::new();
    let input = r#"
        DEF read_bme
          LIT 0x76 I2C_ADDR
          BME_READ
        END

        DEF check_alert
          DUP FLIT 35.0 GT
          IF
            STR "cactus/zone1/alert" STR "high_temp" MQTT_PUB
          ELSE
            STR "cactus/zone1/temp" SWAP F>STR MQTT_PUB
          THEN
        END

        DEF pump_mist
          LIT 12 LIT 1 GPIO_WRITE
          LIT 5000 DELAY_MS
          LIT 12 LIT 0 GPIO_WRITE
        END

        TASK sensor_loop
          EVERY 30000
            read_bme
            check_alert
          ENDEVERY
        ENDTASK

        TASK heartbeat
          EVERY 60000
            MILLIS STR "cactus/zone1/heartbeat" SWAP I>STR MQTT_PUB
          ENDEVERY
        ENDTASK

        TASK main
          START sensor_loop
          START heartbeat
        ENDTASK
    "#;

    let result = parse(input, &mut strings, &mut dict);
    assert!(result.is_ok(), "Cactus monitor should parse: {:?}", result.err());

    let result = result.unwrap();
    assert!(result.entry.is_some(), "main entry should be found");

    // Verify START ops exist
    let mut start_count = 0;
    for i in 0..result.len {
        if let Op::Start(_) = result.ops[i] {
            start_count += 1;
        }
    }
    assert_eq!(start_count, 2, "Should have 2 START ops");
}
