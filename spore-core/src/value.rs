//! Tagged stack cell — runtime type safety with minimal overhead.

/// A tagged value on the stack.
///
/// 8 bytes: 4-byte discriminant + 4-byte payload.
/// At 64 cells per stack, that's 512 bytes per task stack.
#[derive(Clone, Copy, Debug)]
pub enum Value {
    /// Signed 32-bit integer — GPIO pins, counts, raw ADC, addresses.
    I(i32),
    /// 32-bit float — sensor readings, calibrated values.
    F(f32),
    /// Boolean — flags, conditions.
    B(bool),
    /// String — index into StringPool.
    S(u16),
    /// Buffer — index into buffer pool (for raw I2C/SPI).
    Buf(u16),
}

impl Value {
    #[inline(always)]
    pub fn as_int(self) -> i32 {
        match self {
            Value::I(v) => v,
            Value::F(v) => v as i32,
            Value::B(v) => v as i32,
            _ => 0,
        }
    }

    #[inline(always)]
    pub fn as_float(self) -> f32 {
        match self {
            Value::F(v) => v,
            Value::I(v) => v as f32,
            Value::B(v) => if v { 1.0 } else { 0.0 },
            _ => 0.0,
        }
    }

    #[inline(always)]
    pub fn as_bool(self) -> bool {
        match self {
            Value::B(v) => v,
            Value::I(v) => v != 0,
            Value::F(v) => v != 0.0,
            _ => false,
        }
    }

    #[inline(always)]
    pub fn as_str_index(self) -> Option<u16> {
        match self {
            Value::S(idx) => Some(idx),
            _ => None,
        }
    }

    #[inline(always)]
    pub fn as_buf_index(self) -> Option<u16> {
        match self {
            Value::Buf(idx) => Some(idx),
            _ => None,
        }
    }

    /// Returns true if either value is a float (for type promotion).
    #[inline(always)]
    pub fn either_float(a: Value, b: Value) -> bool {
        matches!(a, Value::F(_)) || matches!(b, Value::F(_))
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::I(a), Value::I(b)) => a == b,
            (Value::F(a), Value::F(b)) => a == b,
            (Value::B(a), Value::B(b)) => a == b,
            (Value::S(a), Value::S(b)) => a == b,
            (Value::Buf(a), Value::Buf(b)) => a == b,
            // Cross-type: promote
            (Value::I(a), Value::F(b)) => (*a as f32) == *b,
            (Value::F(a), Value::I(b)) => *a == (*b as f32),
            _ => false,
        }
    }
}
