//! The Spore virtual machine — step/run loop and opcode dispatch.

use crate::buffer::BufferPool;
use crate::op::Op;
use crate::platform::Platform;
use crate::stack::Stack;
use crate::strings::StringPool;
use crate::value::Value;
use crate::{VmAction, VmError};

/// VM execution state after a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepResult {
    /// Executed one instruction, ready for more.
    Continue,
    /// VM halted (HALT opcode or end of program).
    Halted,
    /// VM yielded to scheduler.
    Yielded,
    /// VM yielded permanently (event-driven only).
    YieldedForever,
    /// Error occurred.
    Error(VmError),
}

/// The Spore VM.
///
/// Generic over:
/// - `P`: Platform backend
/// - `DS`: Data stack depth (default 64)
/// - `RS`: Return stack depth (default 32)
/// - `STR_BYTES`: String pool byte capacity (default 2048)
/// - `STR_COUNT`: Max interned strings (default 128)
/// - `BUF_BYTES`: Buffer pool byte capacity (default 1024)
/// - `BUF_COUNT`: Max allocated buffers (default 32)
pub struct Vm<
    P: Platform,
    const DS: usize = 64,
    const RS: usize = 32,
    const STR_BYTES: usize = 2048,
    const STR_COUNT: usize = 128,
    const BUF_BYTES: usize = 1024,
    const BUF_COUNT: usize = 32,
> {
    pub ds: Stack<DS>,
    pub rs: Stack<RS>,
    pub ip: usize,
    pub program: [Op; 1024],
    pub program_len: usize,
    pub vars: [Value; 64],
    pub strings: StringPool<STR_BYTES, STR_COUNT>,
    pub buffers: BufferPool<BUF_BYTES, BUF_COUNT>,
    pub platform: P,
    pub halted: bool,
    /// TIMES loop counter stack (nested TIMES support).
    pub times_stack: [u32; 8],
    pub times_sp: usize,
    /// EVERY state: last tick per nesting level. Per-task — scheduler
    /// swaps these along with stacks.
    pub every_last: [u32; 8],
    pub every_sp: usize,
    /// Pending actions for the scheduler to process.
    actions: [Option<VmAction>; 8],
    action_count: usize,
}

impl<
        P: Platform,
        const DS: usize,
        const RS: usize,
        const STR_BYTES: usize,
        const STR_COUNT: usize,
        const BUF_BYTES: usize,
        const BUF_COUNT: usize,
    > Vm<P, DS, RS, STR_BYTES, STR_COUNT, BUF_BYTES, BUF_COUNT>
{
    pub fn new(platform: P) -> Self {
        Self {
            ds: Stack::new(),
            rs: Stack::new(),
            ip: 0,
            program: [Op::Nop; 1024],
            program_len: 0,
            vars: [Value::I(0); 64],
            strings: StringPool::new(),
            buffers: BufferPool::new(),
            platform,
            halted: false,
            times_stack: [0; 8],
            times_sp: 0,
            every_last: [0; 8],
            every_sp: 0,
            actions: [None; 8],
            action_count: 0,
        }
    }

    pub fn load(&mut self, ops: &[Op]) {
        let len = ops.len().min(self.program.len());
        self.program[..len].copy_from_slice(&ops[..len]);
        self.program_len = len;
        self.ip = 0;
        self.halted = false;
        self.ds.clear();
        self.rs.clear();
        self.vars = [Value::I(0); 64];
        self.times_sp = 0;
        self.every_sp = 0;
        self.action_count = 0;
    }

    /// Execute one instruction.
    pub fn step(&mut self) -> StepResult {
        if self.halted || self.ip >= self.program_len {
            return StepResult::Halted;
        }

        let op = self.program[self.ip];
        self.ip += 1;

        match self.dispatch(op) {
            Ok(result) => result,
            Err(e) => {
                self.halted = true;
                StepResult::Error(e)
            }
        }
    }

    /// Run until halt, yield, or error.
    pub fn run(&mut self) -> StepResult {
        loop {
            let result = self.step();
            match result {
                StepResult::Continue => continue,
                _ => return result,
            }
        }
    }

    /// Run up to `max_steps` instructions.
    pub fn run_steps(&mut self, max_steps: u32) -> StepResult {
        for _ in 0..max_steps {
            let result = self.step();
            match result {
                StepResult::Continue => continue,
                _ => return result,
            }
        }
        StepResult::Continue
    }

    /// Push a pending action for the scheduler.
    fn push_action(&mut self, action: VmAction) -> Result<(), VmError> {
        if self.action_count >= self.actions.len() {
            return Err(VmError::TooManyEvents);
        }
        self.actions[self.action_count] = Some(action);
        self.action_count += 1;
        Ok(())
    }

    /// Drain pending actions. Returns an iterator of actions and clears the queue.
    pub fn drain_actions(&mut self) -> ActionDrain {
        let count = self.action_count;
        self.action_count = 0;
        ActionDrain {
            actions: self.actions,
            pos: 0,
            count,
        }
    }

    fn dispatch(&mut self, op: Op) -> Result<StepResult, VmError> {
        match op {
            Op::Nop => {}
            Op::Halt => {
                self.halted = true;
                return Ok(StepResult::Halted);
            }

            // --- Literals ---
            Op::Lit(v) => self.ds.push(Value::I(v))?,
            Op::FLit(v) => self.ds.push(Value::F(v))?,
            Op::BLit(v) => self.ds.push(Value::B(v))?,
            Op::SLit(v) => self.ds.push(Value::S(v))?,

            // --- Stack manipulation ---
            Op::Drop => {
                self.ds.pop()?;
            }
            Op::Dup => {
                let v = self.ds.peek()?;
                self.ds.push(v)?;
            }
            Op::Swap => {
                let a = self.ds.pop()?;
                let b = self.ds.pop()?;
                self.ds.push(a)?;
                self.ds.push(b)?;
            }
            Op::Over => {
                let a = self.ds.pop()?;
                let b = self.ds.peek()?;
                self.ds.push(a)?;
                self.ds.push(b)?;
            }
            Op::Rot => {
                let c = self.ds.pop()?;
                let b = self.ds.pop()?;
                let a = self.ds.pop()?;
                self.ds.push(b)?;
                self.ds.push(c)?;
                self.ds.push(a)?;
            }
            Op::Nip => {
                let a = self.ds.pop()?;
                self.ds.pop()?;
                self.ds.push(a)?;
            }
            Op::Tuck => {
                let b = self.ds.pop()?;
                let a = self.ds.pop()?;
                self.ds.push(b)?;
                self.ds.push(a)?;
                self.ds.push(b)?;
            }
            Op::TwoDup => {
                let b = self.ds.peek_at(0)?;
                let a = self.ds.peek_at(1)?;
                self.ds.push(a)?;
                self.ds.push(b)?;
            }
            Op::TwoDrop => {
                self.ds.pop()?;
                self.ds.pop()?;
            }
            Op::Depth => {
                let d = self.ds.depth() as i32;
                self.ds.push(Value::I(d))?;
            }

            // --- Arithmetic ---
            Op::Add => self.binary_arith(|a, b| a.wrapping_add(b), |a, b| a + b)?,
            Op::Sub => self.binary_arith(|a, b| a.wrapping_sub(b), |a, b| a - b)?,
            Op::Mul => self.binary_arith(|a, b| a.wrapping_mul(b), |a, b| a * b)?,
            Op::Div => {
                let b = self.ds.pop()?;
                let a = self.ds.pop()?;
                if Value::either_float(a, b) {
                    let fb = b.as_float();
                    if fb == 0.0 {
                        return Err(VmError::DivisionByZero);
                    }
                    self.ds.push(Value::F(a.as_float() / fb))?;
                } else {
                    let ib = b.as_int();
                    if ib == 0 {
                        return Err(VmError::DivisionByZero);
                    }
                    self.ds.push(Value::I(a.as_int().wrapping_div(ib)))?;
                }
            }
            Op::Mod => {
                let b = self.ds.pop()?.as_int();
                let a = self.ds.pop()?.as_int();
                if b == 0 {
                    return Err(VmError::DivisionByZero);
                }
                self.ds.push(Value::I(a.wrapping_rem(b)))?;
            }
            Op::Abs => {
                let v = self.ds.pop()?;
                match v {
                    Value::F(f) => self.ds.push(Value::F(if f < 0.0 { -f } else { f }))?,
                    _ => self.ds.push(Value::I(v.as_int().wrapping_abs()))?,
                }
            }
            Op::Min => {
                let b = self.ds.pop()?;
                let a = self.ds.pop()?;
                if Value::either_float(a, b) {
                    let fa = a.as_float();
                    let fb = b.as_float();
                    self.ds.push(Value::F(if fa < fb { fa } else { fb }))?;
                } else {
                    let ia = a.as_int();
                    let ib = b.as_int();
                    self.ds.push(Value::I(if ia < ib { ia } else { ib }))?;
                }
            }
            Op::Max => {
                let b = self.ds.pop()?;
                let a = self.ds.pop()?;
                if Value::either_float(a, b) {
                    let fa = a.as_float();
                    let fb = b.as_float();
                    self.ds.push(Value::F(if fa > fb { fa } else { fb }))?;
                } else {
                    let ia = a.as_int();
                    let ib = b.as_int();
                    self.ds.push(Value::I(if ia > ib { ia } else { ib }))?;
                }
            }
            Op::Neg => {
                let v = self.ds.pop()?;
                match v {
                    Value::F(f) => self.ds.push(Value::F(-f))?,
                    _ => self.ds.push(Value::I(v.as_int().wrapping_neg()))?,
                }
            }

            // --- Comparison ---
            Op::Eq => self.binary_cmp(|a, b| a == b, |a, b| a == b)?,
            Op::Neq => self.binary_cmp(|a, b| a != b, |a, b| a != b)?,
            Op::Gt => self.binary_cmp(|a, b| a > b, |a, b| a > b)?,
            Op::Lt => self.binary_cmp(|a, b| a < b, |a, b| a < b)?,
            Op::Gte => self.binary_cmp(|a, b| a >= b, |a, b| a >= b)?,
            Op::Lte => self.binary_cmp(|a, b| a <= b, |a, b| a <= b)?,

            // --- Logic ---
            Op::And => {
                let b = self.ds.pop()?;
                let a = self.ds.pop()?;
                match (&a, &b) {
                    (Value::B(_), Value::B(_)) => {
                        self.ds.push(Value::B(a.as_bool() && b.as_bool()))?;
                    }
                    _ => {
                        self.ds.push(Value::I(a.as_int() & b.as_int()))?;
                    }
                }
            }
            Op::Or => {
                let b = self.ds.pop()?;
                let a = self.ds.pop()?;
                match (&a, &b) {
                    (Value::B(_), Value::B(_)) => {
                        self.ds.push(Value::B(a.as_bool() || b.as_bool()))?;
                    }
                    _ => {
                        self.ds.push(Value::I(a.as_int() | b.as_int()))?;
                    }
                }
            }
            Op::Not => {
                let v = self.ds.pop()?;
                match v {
                    Value::B(b) => self.ds.push(Value::B(!b))?,
                    _ => self.ds.push(Value::I(!v.as_int()))?,
                }
            }
            Op::Xor => {
                let b = self.ds.pop()?.as_int();
                let a = self.ds.pop()?.as_int();
                self.ds.push(Value::I(a ^ b))?;
            }
            Op::Shl => {
                let n = self.ds.pop()?.as_int();
                let a = self.ds.pop()?.as_int();
                self.ds.push(Value::I(a.wrapping_shl(n as u32)))?;
            }
            Op::Shr => {
                let n = self.ds.pop()?.as_int();
                let a = self.ds.pop()?.as_int();
                self.ds.push(Value::I(a.wrapping_shr(n as u32)))?;
            }

            // --- Type conversion ---
            Op::ItoF => {
                let v = self.ds.pop()?.as_int();
                self.ds.push(Value::F(v as f32))?;
            }
            Op::FtoI => {
                let v = self.ds.pop()?.as_float();
                self.ds.push(Value::I(v as i32))?;
            }
            Op::ItoStr => {
                let v = self.ds.pop()?.as_int();
                let mut buf = [0u8; 12];
                let s = format_i32(v, &mut buf);
                let idx = self.strings.intern(s)?;
                self.ds.push(Value::S(idx))?;
            }
            Op::FtoStr => {
                let v = self.ds.pop()?.as_float();
                let mut buf = [0u8; 20];
                let s = format_f32(v, &mut buf);
                let idx = self.strings.intern(s)?;
                self.ds.push(Value::S(idx))?;
            }

            // --- Control flow ---
            Op::If(offset) => {
                let cond = self.ds.pop()?.as_bool();
                if !cond {
                    self.ip = offset as usize;
                }
            }
            Op::Else(offset) => {
                self.ip = offset as usize;
            }
            Op::Then => {}

            Op::Loop(_end_offset) => {
                // Loop start marker. BREAK reads end_offset from here.
            }
            Op::EndLoop(start_offset) => {
                self.ip = start_offset as usize;
            }
            Op::Break(loop_start) => {
                // Read the end offset from the Loop opcode (resolved at parse time).
                if let Op::Loop(end_off) = self.program[loop_start as usize] {
                    self.ip = end_off as usize;
                } else {
                    return Err(VmError::ParseError);
                }
            }

            Op::Times(end_offset) => {
                let count = self.ds.pop()?.as_int();
                if count <= 0 {
                    self.ip = end_offset as usize;
                } else {
                    if self.times_sp >= self.times_stack.len() {
                        return Err(VmError::StackOverflow);
                    }
                    self.times_stack[self.times_sp] = count as u32;
                    self.times_sp += 1;
                }
            }
            Op::EndTimes(start_offset) => {
                if self.times_sp == 0 {
                    return Err(VmError::StackUnderflow);
                }
                self.times_stack[self.times_sp - 1] -= 1;
                if self.times_stack[self.times_sp - 1] > 0 {
                    self.ip = start_offset as usize;
                } else {
                    self.times_sp -= 1;
                }
            }

            Op::Begin => {}
            Op::Until(begin_offset) => {
                let cond = self.ds.pop()?.as_bool();
                if !cond {
                    self.ip = begin_offset as usize;
                }
            }

            // --- Variables ---
            Op::Store(slot) => {
                let v = self.ds.pop()?;
                let s = slot as usize;
                if s >= self.vars.len() {
                    return Err(VmError::UnknownWord);
                }
                self.vars[s] = v;
            }
            Op::Fetch(slot) => {
                let s = slot as usize;
                if s >= self.vars.len() {
                    return Err(VmError::UnknownWord);
                }
                self.ds.push(self.vars[s])?;
            }

            // --- Words ---
            Op::Call(offset) => {
                self.rs.push(Value::I(self.ip as i32))?;
                self.ip = offset as usize;
            }
            Op::Return => {
                let ret = self.rs.pop()?;
                self.ip = ret.as_int() as usize;
            }

            // --- Tasks ---
            Op::Yield => return Ok(StepResult::Yielded),
            Op::YieldForever => return Ok(StepResult::YieldedForever),
            Op::Start(name_idx) => {
                self.push_action(VmAction::StartTask(name_idx))?;
            }
            Op::Stop(name_idx) => {
                self.push_action(VmAction::StopTask(name_idx))?;
            }
            Op::Every(interval_ms, end_offset) => {
                let now = self.platform.millis().unwrap_or(0);
                if self.every_sp >= self.every_last.len() {
                    return Err(VmError::StackOverflow);
                }
                let last = self.every_last[self.every_sp];
                if now.wrapping_sub(last) < interval_ms {
                    // Not time yet — jump past ENDEVERY using resolved offset
                    self.ip = end_offset as usize;
                } else {
                    self.every_last[self.every_sp] = now;
                    self.every_sp += 1;
                }
            }
            Op::EndEvery => {
                if self.every_sp > 0 {
                    self.every_sp -= 1;
                }
            }

            // --- Events ---
            Op::On(event_id, word_offset) => {
                self.push_action(VmAction::BindEvent {
                    event_id,
                    word_offset,
                })?;
            }
            Op::EmitEvent(event_id) => {
                self.push_action(VmAction::EmitEvent(event_id))?;
            }

            // --- Platform: GPIO ---
            Op::PGpioMode => {
                let mode = self.ds.pop()?.as_int();
                let pin = self.ds.pop()?.as_int();
                self.platform.gpio_mode(pin, mode)?;
            }
            Op::PGpioWrite => {
                let val = self.ds.pop()?.as_int();
                let pin = self.ds.pop()?.as_int();
                self.platform.gpio_write(pin, val)?;
            }
            Op::PGpioRead => {
                let pin = self.ds.pop()?.as_int();
                let val = self.platform.gpio_read(pin)?;
                self.ds.push(Value::I(val))?;
            }
            Op::PGpioToggle => {
                let pin = self.ds.pop()?.as_int();
                self.platform.gpio_toggle(pin)?;
            }
            Op::PAdcRead => {
                let pin = self.ds.pop()?.as_int();
                let val = self.platform.adc_read(pin)?;
                self.ds.push(Value::I(val))?;
            }
            Op::PPwmInit => {
                let freq = self.ds.pop()?.as_int();
                let pin = self.ds.pop()?.as_int();
                self.platform.pwm_init(pin, freq)?;
            }
            Op::PPwmDuty => {
                let duty = self.ds.pop()?.as_int();
                let pin = self.ds.pop()?.as_int();
                self.platform.pwm_duty(pin, duty)?;
            }

            // --- Platform: I2C ---
            Op::PI2cAddr => {
                let addr = self.ds.pop()?.as_int();
                self.platform.i2c_set_addr(addr)?;
            }
            Op::PI2cWrite => {
                let byte = self.ds.pop()?.as_int() as u8;
                self.platform.i2c_write_byte(byte)?;
            }
            Op::PI2cRead => {
                let byte = self.platform.i2c_read_byte()?;
                self.ds.push(Value::I(byte as i32))?;
            }
            Op::PI2cWriteBuf => {
                let buf_idx = self.ds.pop()?.as_buf_index().ok_or(VmError::TypeMismatch)?;
                let data = self.buffers.get(buf_idx).ok_or(VmError::TypeMismatch)?;
                self.platform.i2c_write_buf(data)?;
            }
            Op::PI2cReadBuf => {
                let len = self.ds.pop()?.as_int() as usize;
                let buf_idx = self.buffers.alloc(len)?;
                let data = self.buffers.get_mut(buf_idx).ok_or(VmError::TypeMismatch)?;
                self.platform.i2c_read_buf(data)?;
                self.ds.push(Value::Buf(buf_idx))?;
            }
            Op::PBmeRead => {
                let (temp, hum, pres) = self.platform.bme_read()?;
                self.ds.push(Value::F(temp))?;
                self.ds.push(Value::F(hum))?;
                self.ds.push(Value::F(pres))?;
            }

            // --- Platform: SPI ---
            Op::PSpiInit => {
                let cs = self.ds.pop()?.as_int();
                let miso = self.ds.pop()?.as_int();
                let mosi = self.ds.pop()?.as_int();
                let clk = self.ds.pop()?.as_int();
                self.platform.spi_init(clk, mosi, miso, cs)?;
            }
            Op::PSpiTransfer => {
                let buf_in_idx =
                    self.ds.pop()?.as_buf_index().ok_or(VmError::TypeMismatch)?;
                let in_data = self.buffers.get(buf_in_idx).ok_or(VmError::TypeMismatch)?;
                let len = in_data.len();
                // Copy input to a temp buffer to avoid double-borrow
                let mut tmp = [0u8; 256];
                let copy_len = len.min(tmp.len());
                tmp[..copy_len].copy_from_slice(&in_data[..copy_len]);
                let buf_out_idx = self.buffers.alloc(copy_len)?;
                let out_data =
                    self.buffers.get_mut(buf_out_idx).ok_or(VmError::TypeMismatch)?;
                self.platform.spi_transfer(&tmp[..copy_len], out_data)?;
                self.ds.push(Value::Buf(buf_out_idx))?;
            }

            // --- Platform: WiFi ---
            Op::PWifiConnect => {
                let pass_idx = self.ds.pop()?.as_str_index().ok_or(VmError::TypeMismatch)?;
                let ssid_idx = self.ds.pop()?.as_str_index().ok_or(VmError::TypeMismatch)?;
                let ssid = self.strings.get(ssid_idx).ok_or(VmError::UnknownWord)?;
                let pass = self.strings.get(pass_idx).ok_or(VmError::UnknownWord)?;
                self.platform.wifi_connect(ssid, pass)?;
            }
            Op::PWifiStatus => {
                let status = self.platform.wifi_status()?;
                self.ds.push(Value::I(status))?;
            }
            Op::PWifiDisconnect => {
                self.platform.wifi_disconnect()?;
            }
            Op::PWifiIp => {
                let ip = self.platform.wifi_ip()?;
                self.ds.push(Value::I(ip))?;
            }

            // --- Platform: BLE ---
            Op::PBleInit => self.platform.ble_init()?,
            Op::PBleAdvertise => {
                let name_idx = self.ds.pop()?.as_str_index().ok_or(VmError::TypeMismatch)?;
                let name = self.strings.get(name_idx).ok_or(VmError::UnknownWord)?;
                self.platform.ble_advertise(name)?;
            }
            Op::PBleStopAdv => self.platform.ble_stop_adv()?,
            Op::PBleNotify => {
                let data_idx = self.ds.pop()?.as_str_index().ok_or(VmError::TypeMismatch)?;
                let handle = self.ds.pop()?.as_int();
                let data = self.strings.get(data_idx).ok_or(VmError::UnknownWord)?;
                self.platform.ble_notify(handle, data)?;
            }
            Op::PBleRead => {
                let handle = self.ds.pop()?.as_int();
                let buf_idx = self.platform.ble_read(handle)?;
                self.ds.push(Value::Buf(buf_idx))?;
            }

            // --- Platform: MQTT ---
            Op::PMqttInit => {
                let port = self.ds.pop()?.as_int();
                let broker_idx =
                    self.ds.pop()?.as_str_index().ok_or(VmError::TypeMismatch)?;
                let broker = self.strings.get(broker_idx).ok_or(VmError::UnknownWord)?;
                self.platform.mqtt_init(broker, port)?;
            }
            Op::PMqttPub => {
                let payload_idx =
                    self.ds.pop()?.as_str_index().ok_or(VmError::TypeMismatch)?;
                let topic_idx =
                    self.ds.pop()?.as_str_index().ok_or(VmError::TypeMismatch)?;
                let topic = self.strings.get(topic_idx).ok_or(VmError::UnknownWord)?;
                let payload =
                    self.strings.get(payload_idx).ok_or(VmError::UnknownWord)?;
                self.platform.mqtt_pub(topic, payload)?;
            }
            Op::PMqttSub => {
                let topic_idx =
                    self.ds.pop()?.as_str_index().ok_or(VmError::TypeMismatch)?;
                let topic = self.strings.get(topic_idx).ok_or(VmError::UnknownWord)?;
                self.platform.mqtt_sub(topic)?;
            }
            Op::PMqttUnsub => {
                let topic_idx =
                    self.ds.pop()?.as_str_index().ok_or(VmError::TypeMismatch)?;
                let topic = self.strings.get(topic_idx).ok_or(VmError::UnknownWord)?;
                self.platform.mqtt_unsub(topic)?;
            }

            // --- Platform: System ---
            Op::PDelayMs => {
                let ms = self.ds.pop()?.as_int() as u32;
                self.platform.delay_ms(ms)?;
            }
            Op::PMillis => {
                let ms = self.platform.millis()?;
                self.ds.push(Value::I(ms as i32))?;
            }
            Op::PDeepSleep => {
                let secs = self.ds.pop()?.as_int() as u32;
                self.platform.deep_sleep(secs)?;
            }
            Op::PReboot => {
                self.platform.reboot()?;
            }
            Op::PNvsGet => {
                let key_idx = self.ds.pop()?.as_str_index().ok_or(VmError::TypeMismatch)?;
                let key = self.strings.get(key_idx).ok_or(VmError::UnknownWord)?;
                let val = self.platform.nvs_get(key)?;
                self.ds.push(Value::I(val))?;
            }
            Op::PNvsSet => {
                let val = self.ds.pop()?.as_int();
                let key_idx = self.ds.pop()?.as_str_index().ok_or(VmError::TypeMismatch)?;
                let key = self.strings.get(key_idx).ok_or(VmError::UnknownWord)?;
                self.platform.nvs_set(key, val)?;
            }
            Op::PHeapFree => {
                let bytes = self.platform.heap_free()?;
                self.ds.push(Value::I(bytes as i32))?;
            }
            Op::PLog => {
                let msg_idx = self.ds.pop()?.as_str_index().ok_or(VmError::TypeMismatch)?;
                let msg = self.strings.get(msg_idx).ok_or(VmError::UnknownWord)?;
                self.platform.log(msg)?;
            }

            // --- Platform: OTA ---
            Op::POtaRecv => self.platform.ota_recv()?,
            Op::POtaLoad => {
                let prog_idx =
                    self.ds.pop()?.as_str_index().ok_or(VmError::TypeMismatch)?;
                let prog = self.strings.get(prog_idx).ok_or(VmError::UnknownWord)?;
                self.platform.ota_load(prog)?;
            }
        }

        Ok(StepResult::Continue)
    }

    /// Binary arithmetic with type promotion.
    fn binary_arith(
        &mut self,
        int_op: fn(i32, i32) -> i32,
        float_op: fn(f32, f32) -> f32,
    ) -> Result<(), VmError> {
        let b = self.ds.pop()?;
        let a = self.ds.pop()?;
        if Value::either_float(a, b) {
            self.ds
                .push(Value::F(float_op(a.as_float(), b.as_float())))?;
        } else {
            self.ds
                .push(Value::I(int_op(a.as_int(), b.as_int())))?;
        }
        Ok(())
    }

    /// Binary comparison with type promotion.
    fn binary_cmp(
        &mut self,
        int_op: fn(i32, i32) -> bool,
        float_op: fn(f32, f32) -> bool,
    ) -> Result<(), VmError> {
        let b = self.ds.pop()?;
        let a = self.ds.pop()?;
        let result = if Value::either_float(a, b) {
            float_op(a.as_float(), b.as_float())
        } else {
            int_op(a.as_int(), b.as_int())
        };
        self.ds.push(Value::B(result))?;
        Ok(())
    }
}

/// Iterator over drained VM actions.
pub struct ActionDrain {
    actions: [Option<VmAction>; 8],
    pos: usize,
    count: usize,
}

impl Iterator for ActionDrain {
    type Item = VmAction;

    fn next(&mut self) -> Option<VmAction> {
        if self.pos >= self.count {
            return None;
        }
        let action = self.actions[self.pos].take();
        self.pos += 1;
        action
    }
}

/// Format i32 into a fixed buffer, return the string slice.
fn format_i32(v: i32, buf: &mut [u8; 12]) -> &str {
    let negative = v < 0;
    // Work in u32 to handle i32::MIN correctly (wrapping_neg of MIN is MIN).
    let mut n: u32 = if negative { (v as u32).wrapping_neg() } else { v as u32 };
    let mut pos = buf.len();

    if n == 0 {
        pos -= 1;
        buf[pos] = b'0';
    } else {
        while n > 0 {
            pos -= 1;
            buf[pos] = b'0' + (n % 10) as u8;
            n /= 10;
        }
    }

    if negative {
        pos -= 1;
        buf[pos] = b'-';
    }

    unsafe { core::str::from_utf8_unchecked(&buf[pos..]) }
}

/// Format f32 into a fixed buffer with 2 decimal places.
fn format_f32(v: f32, buf: &mut [u8; 20]) -> &str {
    let negative = v < 0.0;
    let v = if negative { -v } else { v };
    let mut integer_part = v as u32;
    let mut frac_part = ((v - integer_part as f32) * 100.0 + 0.5) as u32;

    // Handle rounding carry (e.g., 1.995 → frac rounds to 100)
    if frac_part >= 100 {
        integer_part += 1;
        frac_part = 0;
    }

    let mut pos = buf.len();

    let d1 = (frac_part % 10) as u8;
    let d0 = ((frac_part / 10) % 10) as u8;
    pos -= 1;
    buf[pos] = b'0' + d1;
    pos -= 1;
    buf[pos] = b'0' + d0;
    pos -= 1;
    buf[pos] = b'.';

    let mut n = integer_part;
    if n == 0 {
        pos -= 1;
        buf[pos] = b'0';
    } else {
        while n > 0 {
            pos -= 1;
            buf[pos] = b'0' + (n % 10) as u8;
            n /= 10;
        }
    }

    if negative {
        pos -= 1;
        buf[pos] = b'-';
    }

    unsafe { core::str::from_utf8_unchecked(&buf[pos..]) }
}
