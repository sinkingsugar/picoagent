//! Fixed-size, const-generic stack. No allocations.

use crate::value::Value;
use crate::VmError;

pub struct Stack<const N: usize> {
    data: [Value; N],
    sp: usize,
}

impl<const N: usize> Stack<N> {
    pub const fn new() -> Self {
        Self {
            data: [Value::I(0); N],
            sp: 0,
        }
    }

    #[inline(always)]
    pub fn push(&mut self, v: Value) -> Result<(), VmError> {
        if self.sp >= N {
            return Err(VmError::StackOverflow);
        }
        self.data[self.sp] = v;
        self.sp += 1;
        Ok(())
    }

    #[inline(always)]
    pub fn pop(&mut self) -> Result<Value, VmError> {
        if self.sp == 0 {
            return Err(VmError::StackUnderflow);
        }
        self.sp -= 1;
        Ok(self.data[self.sp])
    }

    #[inline(always)]
    pub fn peek(&self) -> Result<Value, VmError> {
        if self.sp == 0 {
            return Err(VmError::StackUnderflow);
        }
        Ok(self.data[self.sp - 1])
    }

    #[inline(always)]
    pub fn peek_at(&self, depth: usize) -> Result<Value, VmError> {
        if depth >= self.sp {
            return Err(VmError::StackUnderflow);
        }
        Ok(self.data[self.sp - 1 - depth])
    }

    pub fn depth(&self) -> usize {
        self.sp
    }

    pub fn clear(&mut self) {
        self.sp = 0;
    }
}
