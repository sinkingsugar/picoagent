//! Opcodes — compiled from the token stream, stored as a flat array.

/// A single VM instruction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    Nop,
    Halt,

    // --- Literals ---
    Lit(i32),
    FLit(f32),
    BLit(bool),
    SLit(u16), // string pool index

    // --- Stack manipulation ---
    Drop,
    Dup,
    Swap,
    Over,
    Rot,
    Nip,
    Tuck,
    TwoDup,
    TwoDrop,
    Depth,

    // --- Arithmetic (type-promoting) ---
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Abs,
    Min,
    Max,
    Neg,

    // --- Comparison ---
    Eq,
    Neq,
    Gt,
    Lt,
    Gte,
    Lte,

    // --- Logic ---
    And,
    Or,
    Not,
    Xor,
    Shl,
    Shr,

    // --- Type conversion ---
    ItoF,
    FtoI,
    ItoStr,
    FtoStr,

    // --- Control flow (offsets resolved at parse time) ---
    /// Jump to offset if top-of-stack is false.
    If(u16),
    /// Unconditional jump (to THEN).
    Else(u16),
    /// Nop marker — target of If/Else jumps.
    Then,
    /// Loop start. Stores offset to after ENDLOOP for BREAK.
    Loop(u16),
    /// Jump back to Loop start.
    EndLoop(u16),
    /// Exit innermost loop. Offset resolved at parse time.
    Break(u16),
    /// Pop count, jump to offset if count is 0.
    Times(u16),
    /// Decrement counter, jump back if count > 0.
    EndTimes(u16),
    /// Marks loop start (for BEGIN...UNTIL).
    Begin,
    /// Pop flag, jump back to Begin if false.
    Until(u16),

    // --- Variables ---
    /// Store top-of-stack into variable slot.
    Store(u16),
    /// Fetch variable slot onto stack.
    Fetch(u16),

    // --- Words ---
    /// Call word at program offset.
    Call(u16),
    /// Return from word.
    Return,

    // --- Tasks ---
    Yield,
    YieldForever,
    /// Start a task by name (string pool index).
    Start(u16),
    /// Stop a task by name (string pool index).
    Stop(u16),
    /// Periodic block: interval in ms, marks block start.
    Every(u32),
    EndEvery,

    // --- Events ---
    /// Bind event_id to word at program offset.
    On(u16, u16),
    /// Emit user event.
    EmitEvent(u16),

    // --- Platform: GPIO ---
    PGpioMode,
    PGpioWrite,
    PGpioRead,
    PGpioToggle,
    PAdcRead,
    PPwmInit,
    PPwmDuty,

    // --- Platform: I2C ---
    PI2cAddr,
    PI2cWrite,
    PI2cRead,
    PI2cWriteBuf,
    PI2cReadBuf,
    PBmeRead,

    // --- Platform: SPI ---
    PSpiInit,
    PSpiTransfer,

    // --- Platform: WiFi ---
    PWifiConnect,
    PWifiStatus,
    PWifiDisconnect,
    PWifiIp,

    // --- Platform: BLE ---
    PBleInit,
    PBleAdvertise,
    PBleStopAdv,
    PBleNotify,
    PBleRead,

    // --- Platform: MQTT ---
    PMqttInit,
    PMqttPub,
    PMqttSub,
    PMqttUnsub,

    // --- Platform: System ---
    PDelayMs,
    PMillis,
    PDeepSleep,
    PReboot,
    PNvsGet,
    PNvsSet,
    PHeapFree,
    PLog,

    // --- Platform: OTA ---
    POtaRecv,
    POtaLoad,
}
