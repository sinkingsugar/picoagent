//! Token stream parser — text → Op sequence.
//!
//! Parses the Spore uppercase, space-delimited token format.
//! Resolves control flow offsets, word definitions, and string literals.

use crate::dict::Dict;
use crate::op::Op;
use crate::strings::StringPool;
use crate::VmError;

/// Parse result: program ops and metadata.
#[derive(Debug)]
pub struct ParseResult {
    pub ops: [Op; 1024],
    pub len: usize,
    /// Entry point offset (offset of `main` task body, or 0).
    pub entry: usize,
}

/// Fixup entry for forward references (IF→ELSE/THEN, LOOP→ENDLOOP, etc.)
struct Fixup {
    /// Index into ops array that needs patching.
    op_idx: usize,
    /// What kind of fixup.
    kind: FixupKind,
}

#[derive(Clone, Copy)]
enum FixupKind {
    IfToElseOrThen,
    ElseToThen,
    LoopToEndLoop,
    TimesToEndTimes,
}

/// Parse a Spore token stream into an Op sequence.
///
/// `strings` and `dict` are filled during parsing and should be passed to the VM.
pub fn parse<const SB: usize, const SC: usize, const DN: usize>(
    input: &str,
    strings: &mut StringPool<SB, SC>,
    dict: &mut Dict<DN>,
) -> Result<ParseResult, VmError> {
    let mut ops = [Op::Nop; 1024];
    let mut len: usize = 0;
    let mut fixup_stack: [Fixup; 64] = unsafe { core::mem::zeroed() };
    let mut fixup_sp: usize = 0;
    // For BEGIN...UNTIL: stack of Begin offsets
    let mut begin_stack: [usize; 16] = [0; 16];
    let mut begin_sp: usize = 0;
    // Variable name → slot mapping
    let mut var_names: [u16; 64] = [0xFFFF; 64];
    let mut var_count: usize = 0;
    // Track task/word entry points
    let mut entry: usize = 0;
    // Whether we're inside a DEF or TASK
    let mut in_def = false;
    let mut _def_start: usize = 0;

    let mut tokens = Tokenizer::new(input);

    while let Some(tok) = tokens.next() {
        if len >= ops.len() {
            return Err(VmError::ProgramTooLarge);
        }

        match tok {
            // --- Literals ---
            "LIT" => {
                let n = tokens.next().ok_or(VmError::ParseError)?;
                let v = parse_int(n)?;
                ops[len] = Op::Lit(v);
                len += 1;
            }
            "FLIT" => {
                let n = tokens.next().ok_or(VmError::ParseError)?;
                let v = parse_float(n)?;
                ops[len] = Op::FLit(v);
                len += 1;
            }
            "STR" => {
                let s = tokens.next_string().ok_or(VmError::ParseError)?;
                let idx = strings.intern(s)?;
                ops[len] = Op::SLit(idx);
                len += 1;
            }
            "TRUE" => {
                ops[len] = Op::BLit(true);
                len += 1;
            }
            "FALSE" => {
                ops[len] = Op::BLit(false);
                len += 1;
            }

            // --- Stack ---
            "DUP" => { ops[len] = Op::Dup; len += 1; }
            "DROP" => { ops[len] = Op::Drop; len += 1; }
            "SWAP" => { ops[len] = Op::Swap; len += 1; }
            "OVER" => { ops[len] = Op::Over; len += 1; }
            "ROT" => { ops[len] = Op::Rot; len += 1; }
            "NIP" => { ops[len] = Op::Nip; len += 1; }
            "TUCK" => { ops[len] = Op::Tuck; len += 1; }
            "2DUP" => { ops[len] = Op::TwoDup; len += 1; }
            "2DROP" => { ops[len] = Op::TwoDrop; len += 1; }
            "DEPTH" => { ops[len] = Op::Depth; len += 1; }

            // --- Arithmetic ---
            "ADD" => { ops[len] = Op::Add; len += 1; }
            "SUB" => { ops[len] = Op::Sub; len += 1; }
            "MUL" => { ops[len] = Op::Mul; len += 1; }
            "DIV" => { ops[len] = Op::Div; len += 1; }
            "MOD" => { ops[len] = Op::Mod; len += 1; }
            "ABS" => { ops[len] = Op::Abs; len += 1; }
            "MIN" => { ops[len] = Op::Min; len += 1; }
            "MAX" => { ops[len] = Op::Max; len += 1; }
            "NEG" => { ops[len] = Op::Neg; len += 1; }

            // --- Comparison ---
            "EQ" => { ops[len] = Op::Eq; len += 1; }
            "NEQ" => { ops[len] = Op::Neq; len += 1; }
            "GT" => { ops[len] = Op::Gt; len += 1; }
            "LT" => { ops[len] = Op::Lt; len += 1; }
            "GTE" => { ops[len] = Op::Gte; len += 1; }
            "LTE" => { ops[len] = Op::Lte; len += 1; }

            // --- Logic ---
            "AND" => { ops[len] = Op::And; len += 1; }
            "OR" => { ops[len] = Op::Or; len += 1; }
            "NOT" => { ops[len] = Op::Not; len += 1; }
            "XOR" => { ops[len] = Op::Xor; len += 1; }
            "SHL" => { ops[len] = Op::Shl; len += 1; }
            "SHR" => { ops[len] = Op::Shr; len += 1; }

            // --- Type conversion ---
            "I>F" => { ops[len] = Op::ItoF; len += 1; }
            "F>I" => { ops[len] = Op::FtoI; len += 1; }
            "I>STR" => { ops[len] = Op::ItoStr; len += 1; }
            "F>STR" => { ops[len] = Op::FtoStr; len += 1; }

            // --- Control flow ---
            "IF" => {
                if fixup_sp >= fixup_stack.len() {
                    return Err(VmError::ProgramTooLarge);
                }
                fixup_stack[fixup_sp] = Fixup {
                    op_idx: len,
                    kind: FixupKind::IfToElseOrThen,
                };
                fixup_sp += 1;
                ops[len] = Op::If(0); // placeholder
                len += 1;
            }
            "ELSE" => {
                // Patch the IF to jump here+1 (to skip the else branch)
                if fixup_sp == 0 {
                    return Err(VmError::ParseError);
                }
                fixup_sp -= 1;
                let fixup = &fixup_stack[fixup_sp];
                ops[fixup.op_idx] = Op::If((len + 1) as u16); // IF jumps past ELSE

                // Push ELSE fixup
                if fixup_sp >= fixup_stack.len() {
                    return Err(VmError::ProgramTooLarge);
                }
                fixup_stack[fixup_sp] = Fixup {
                    op_idx: len,
                    kind: FixupKind::ElseToThen,
                };
                fixup_sp += 1;
                ops[len] = Op::Else(0); // placeholder
                len += 1;
            }
            "THEN" => {
                if fixup_sp == 0 {
                    return Err(VmError::ParseError);
                }
                fixup_sp -= 1;
                let fixup = &fixup_stack[fixup_sp];
                match fixup.kind {
                    FixupKind::IfToElseOrThen => {
                        ops[fixup.op_idx] = Op::If(len as u16);
                    }
                    FixupKind::ElseToThen => {
                        ops[fixup.op_idx] = Op::Else(len as u16);
                    }
                    _ => return Err(VmError::ParseError),
                }
                ops[len] = Op::Then;
                len += 1;
            }

            "LOOP" => {
                if fixup_sp >= fixup_stack.len() {
                    return Err(VmError::ProgramTooLarge);
                }
                fixup_stack[fixup_sp] = Fixup {
                    op_idx: len,
                    kind: FixupKind::LoopToEndLoop,
                };
                fixup_sp += 1;
                ops[len] = Op::Loop(0); // placeholder
                len += 1;
            }
            "ENDLOOP" => {
                if fixup_sp == 0 {
                    return Err(VmError::ParseError);
                }
                fixup_sp -= 1;
                let fixup = &fixup_stack[fixup_sp];
                let loop_start = fixup.op_idx;
                // ENDLOOP jumps back to Loop (which is a no-op, then body re-executes)
                ops[len] = Op::EndLoop(loop_start as u16);
                // Patch Loop with offset to after ENDLOOP (for BREAK)
                ops[loop_start] = Op::Loop((len + 1) as u16);
                len += 1;
            }
            "BREAK" => {
                ops[len] = Op::Break;
                len += 1;
            }

            "TIMES" => {
                if fixup_sp >= fixup_stack.len() {
                    return Err(VmError::ProgramTooLarge);
                }
                fixup_stack[fixup_sp] = Fixup {
                    op_idx: len,
                    kind: FixupKind::TimesToEndTimes,
                };
                fixup_sp += 1;
                ops[len] = Op::Times(0); // placeholder
                len += 1;
            }
            "ENDTIMES" => {
                if fixup_sp == 0 {
                    return Err(VmError::ParseError);
                }
                fixup_sp -= 1;
                let fixup = &fixup_stack[fixup_sp];
                let times_start = fixup.op_idx;
                // ENDTIMES jumps back to instruction after Times
                ops[len] = Op::EndTimes((times_start + 1) as u16);
                // Patch Times with offset to after ENDTIMES
                ops[times_start] = Op::Times((len + 1) as u16);
                len += 1;
            }

            "BEGIN" => {
                if begin_sp >= begin_stack.len() {
                    return Err(VmError::ProgramTooLarge);
                }
                begin_stack[begin_sp] = len;
                begin_sp += 1;
                ops[len] = Op::Begin;
                len += 1;
            }
            "UNTIL" => {
                if begin_sp == 0 {
                    return Err(VmError::ParseError);
                }
                begin_sp -= 1;
                let begin_off = begin_stack[begin_sp];
                ops[len] = Op::Until(begin_off as u16);
                len += 1;
            }

            // --- Variables ---
            "VAR" => {
                let name = tokens.next().ok_or(VmError::ParseError)?;
                let name_idx = strings.intern(name)?;
                if var_count >= 64 {
                    return Err(VmError::DictFull);
                }
                var_names[var_count] = name_idx;
                var_count += 1;
            }
            "STORE" => {
                let name = tokens.next().ok_or(VmError::ParseError)?;
                let name_idx = strings.intern(name)?;
                let slot = find_var(&var_names, var_count, name_idx)?;
                ops[len] = Op::Store(slot);
                len += 1;
            }
            "FETCH" => {
                let name = tokens.next().ok_or(VmError::ParseError)?;
                let name_idx = strings.intern(name)?;
                let slot = find_var(&var_names, var_count, name_idx)?;
                ops[len] = Op::Fetch(slot);
                len += 1;
            }

            // --- Word definitions ---
            "DEF" => {
                let name = tokens.next().ok_or(VmError::ParseError)?;
                let name_idx = strings.intern(name)?;
                in_def = true;
                _def_start = len;
                dict.define(name_idx, len as u16)?;
            }
            "END" => {
                if !in_def {
                    return Err(VmError::ParseError);
                }
                ops[len] = Op::Return;
                len += 1;
                in_def = false;
            }

            // --- Tasks ---
            "TASK" => {
                let name = tokens.next().ok_or(VmError::ParseError)?;
                let name_idx = strings.intern(name)?;
                in_def = true;
                _def_start = len;
                dict.define(name_idx, len as u16)?;
                // Check if this is "main"
                if let Some(s) = strings.get(name_idx) {
                    if s == "main" {
                        entry = len;
                    }
                }
            }
            "ENDTASK" => {
                if !in_def {
                    return Err(VmError::ParseError);
                }
                ops[len] = Op::Halt;
                len += 1;
                in_def = false;
            }

            "START" => {
                let name = tokens.next().ok_or(VmError::ParseError)?;
                let name_idx = strings.intern(name)?;
                ops[len] = Op::Start(name_idx);
                len += 1;
            }
            "STOP" => {
                let name = tokens.next().ok_or(VmError::ParseError)?;
                let name_idx = strings.intern(name)?;
                ops[len] = Op::Stop(name_idx);
                len += 1;
            }

            "YIELD" => { ops[len] = Op::Yield; len += 1; }
            "YIELD_FOREVER" => { ops[len] = Op::YieldForever; len += 1; }

            "EVERY" => {
                let interval = tokens.next().ok_or(VmError::ParseError)?;
                let ms = parse_int(interval)? as u32;
                ops[len] = Op::Every(ms);
                len += 1;
            }
            "ENDEVERY" => {
                ops[len] = Op::EndEvery;
                len += 1;
            }

            // --- Events ---
            "ON" => {
                let event = tokens.next().ok_or(VmError::ParseError)?;
                let word = tokens.next().ok_or(VmError::ParseError)?;
                let event_idx = strings.intern(event)?;
                let word_idx = strings.intern(word)?;
                if let Some(offset) = dict.lookup(word_idx) {
                    ops[len] = Op::On(event_idx, offset);
                } else {
                    return Err(VmError::UnknownWord);
                }
                len += 1;
            }
            "EMIT" => {
                let event = tokens.next().ok_or(VmError::ParseError)?;
                let event_idx = strings.intern(event)?;
                ops[len] = Op::EmitEvent(event_idx);
                len += 1;
            }

            // --- Platform ---
            "GPIO_MODE" => { ops[len] = Op::PGpioMode; len += 1; }
            "GPIO_WRITE" => { ops[len] = Op::PGpioWrite; len += 1; }
            "GPIO_READ" => { ops[len] = Op::PGpioRead; len += 1; }
            "GPIO_TOGGLE" => { ops[len] = Op::PGpioToggle; len += 1; }
            "ADC_READ" => { ops[len] = Op::PAdcRead; len += 1; }
            "PWM_INIT" => { ops[len] = Op::PPwmInit; len += 1; }
            "PWM_DUTY" => { ops[len] = Op::PPwmDuty; len += 1; }

            "I2C_ADDR" => { ops[len] = Op::PI2cAddr; len += 1; }
            "I2C_WRITE" => { ops[len] = Op::PI2cWrite; len += 1; }
            "I2C_READ" => { ops[len] = Op::PI2cRead; len += 1; }
            "I2C_WRITE_BUF" => { ops[len] = Op::PI2cWriteBuf; len += 1; }
            "I2C_READ_BUF" => { ops[len] = Op::PI2cReadBuf; len += 1; }
            "BME_READ" => { ops[len] = Op::PBmeRead; len += 1; }

            "SPI_INIT" => { ops[len] = Op::PSpiInit; len += 1; }
            "SPI_TRANSFER" => { ops[len] = Op::PSpiTransfer; len += 1; }

            "WIFI_CONNECT" => { ops[len] = Op::PWifiConnect; len += 1; }
            "WIFI_STATUS" => { ops[len] = Op::PWifiStatus; len += 1; }
            "WIFI_DISCONNECT" => { ops[len] = Op::PWifiDisconnect; len += 1; }
            "WIFI_IP" => { ops[len] = Op::PWifiIp; len += 1; }

            "BLE_INIT" => { ops[len] = Op::PBleInit; len += 1; }
            "BLE_ADVERTISE" => { ops[len] = Op::PBleAdvertise; len += 1; }
            "BLE_STOP_ADV" => { ops[len] = Op::PBleStopAdv; len += 1; }
            "BLE_NOTIFY" => { ops[len] = Op::PBleNotify; len += 1; }
            "BLE_READ" => { ops[len] = Op::PBleRead; len += 1; }

            "MQTT_INIT" => { ops[len] = Op::PMqttInit; len += 1; }
            "MQTT_PUB" => { ops[len] = Op::PMqttPub; len += 1; }
            "MQTT_SUB" => { ops[len] = Op::PMqttSub; len += 1; }
            "MQTT_UNSUB" => { ops[len] = Op::PMqttUnsub; len += 1; }

            "DELAY_MS" => { ops[len] = Op::PDelayMs; len += 1; }
            "MILLIS" => { ops[len] = Op::PMillis; len += 1; }
            "DEEP_SLEEP" => { ops[len] = Op::PDeepSleep; len += 1; }
            "REBOOT" => { ops[len] = Op::PReboot; len += 1; }
            "NVS_GET" => { ops[len] = Op::PNvsGet; len += 1; }
            "NVS_SET" => { ops[len] = Op::PNvsSet; len += 1; }
            "HEAP_FREE" => { ops[len] = Op::PHeapFree; len += 1; }
            "LOG" => { ops[len] = Op::PLog; len += 1; }

            "OTA_RECV" => { ops[len] = Op::POtaRecv; len += 1; }
            "OTA_LOAD" => { ops[len] = Op::POtaLoad; len += 1; }

            "NOP" => { ops[len] = Op::Nop; len += 1; }
            "HALT" => { ops[len] = Op::Halt; len += 1; }

            // Unknown token — try as a word call
            other => {
                let name_idx = strings.intern(other)?;
                if let Some(offset) = dict.lookup(name_idx) {
                    ops[len] = Op::Call(offset);
                    len += 1;
                } else {
                    return Err(VmError::UnknownWord);
                }
            }
        }
    }

    if fixup_sp != 0 || begin_sp != 0 || in_def {
        return Err(VmError::ParseError);
    }

    Ok(ParseResult { ops, len, entry })
}

fn find_var(names: &[u16; 64], count: usize, name_idx: u16) -> Result<u16, VmError> {
    for i in 0..count {
        if names[i] == name_idx {
            return Ok(i as u16);
        }
    }
    Err(VmError::UnknownWord)
}

fn parse_int(s: &str) -> Result<i32, VmError> {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return Err(VmError::ParseError);
    }

    let (negative, start) = if bytes[0] == b'-' {
        (true, 1)
    } else {
        (false, 0)
    };

    // Handle hex: 0x...
    if bytes.len() > start + 2 && bytes[start] == b'0' && (bytes[start + 1] == b'x' || bytes[start + 1] == b'X') {
        let mut result: i32 = 0;
        for &b in &bytes[start + 2..] {
            let digit = match b {
                b'0'..=b'9' => b - b'0',
                b'a'..=b'f' => b - b'a' + 10,
                b'A'..=b'F' => b - b'A' + 10,
                _ => return Err(VmError::ParseError),
            };
            result = result.wrapping_mul(16).wrapping_add(digit as i32);
        }
        return Ok(if negative { -result } else { result });
    }

    let mut result: i32 = 0;
    for &b in &bytes[start..] {
        if b < b'0' || b > b'9' {
            return Err(VmError::ParseError);
        }
        result = result.wrapping_mul(10).wrapping_add((b - b'0') as i32);
    }
    Ok(if negative { -result } else { result })
}

fn parse_float(s: &str) -> Result<f32, VmError> {
    // Simple float parser: [-]digits[.digits]
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return Err(VmError::ParseError);
    }

    let (negative, start) = if bytes[0] == b'-' {
        (true, 1)
    } else {
        (false, 0)
    };

    let mut integer_part: u32 = 0;
    let mut frac_part: u32 = 0;
    let mut frac_divisor: u32 = 1;
    let mut in_frac = false;

    for &b in &bytes[start..] {
        if b == b'.' {
            if in_frac {
                return Err(VmError::ParseError);
            }
            in_frac = true;
            continue;
        }
        if b < b'0' || b > b'9' {
            return Err(VmError::ParseError);
        }
        if in_frac {
            frac_part = frac_part * 10 + (b - b'0') as u32;
            frac_divisor *= 10;
        } else {
            integer_part = integer_part * 10 + (b - b'0') as u32;
        }
    }

    let result = integer_part as f32 + frac_part as f32 / frac_divisor as f32;
    Ok(if negative { -result } else { result })
}

/// Simple tokenizer that handles whitespace and quoted strings.
struct Tokenizer<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> Tokenizer<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, pos: 0 }
    }

    fn skip_whitespace_and_comments(&mut self) {
        let bytes = self.input.as_bytes();
        while self.pos < bytes.len() {
            let b = bytes[self.pos];
            if b == b'\\' {
                // Comment: skip to end of line
                while self.pos < bytes.len() && bytes[self.pos] != b'\n' {
                    self.pos += 1;
                }
            } else if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn next(&mut self) -> Option<&'a str> {
        self.skip_whitespace_and_comments();
        if self.pos >= self.input.len() {
            return None;
        }

        let start = self.pos;
        let bytes = self.input.as_bytes();
        while self.pos < bytes.len()
            && bytes[self.pos] != b' '
            && bytes[self.pos] != b'\t'
            && bytes[self.pos] != b'\n'
            && bytes[self.pos] != b'\r'
        {
            self.pos += 1;
        }

        if self.pos > start {
            Some(&self.input[start..self.pos])
        } else {
            None
        }
    }

    /// Parse a double-quoted string literal. Expects the next non-whitespace
    /// to be `"` and reads until the closing `"`.
    fn next_string(&mut self) -> Option<&'a str> {
        self.skip_whitespace_and_comments();
        let bytes = self.input.as_bytes();
        if self.pos >= bytes.len() || bytes[self.pos] != b'"' {
            return None;
        }
        self.pos += 1; // skip opening "
        let start = self.pos;
        while self.pos < bytes.len() && bytes[self.pos] != b'"' {
            self.pos += 1;
        }
        if self.pos >= bytes.len() {
            return None; // unterminated string
        }
        let s = &self.input[start..self.pos];
        self.pos += 1; // skip closing "
        Some(s)
    }
}
