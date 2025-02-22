use std::fmt::Display;

use crate::{xdr, xdr::ContractEvent, RawVal, Status};
#[cfg(feature = "vm")]
use crate::{
    xdr::{ScUnknownErrorCode, ScVmErrorCode},
    HostError,
};
use log::debug;
use tinyvec::TinyVec;

// TODO: optimize storage on this to use pools / bumpalo / etc.
#[derive(Clone, Debug)]
pub enum HostEvent {
    Contract(ContractEvent),
    Debug(DebugEvent),
}

#[derive(Clone, Debug, Default)]
pub struct Events(pub Vec<HostEvent>);

// Maximum number of topics in a `ContractEvent`. This applies to both
// `Contract` and `System` types of contract events.
pub(crate) const CONTRACT_EVENT_TOPICS_LIMIT: usize = 4;
// Maximum number of bytes in a topic binary.
pub(crate) const TOPIC_BYTES_LENGTH_LIMIT: usize = 32;

impl Events {
    // Records the smallest variant of a debug HostEvent it can, returning the size of the
    // in_args slice (for charging to a budget).
    pub fn record_debug_event(&mut self, de: DebugEvent) -> u64 {
        let len = de.args.len();
        self.0.push(HostEvent::Debug(de));
        len as u64
    }

    // Records a contract HostEvent.
    pub fn record_contract_event(&mut self, ce: ContractEvent) {
        self.0.push(HostEvent::Contract(ce))
    }

    pub fn dump_to_debug_log(&self) {
        for e in self.0.iter() {
            match e {
                HostEvent::Contract(e) => debug!("Contract event: {:?}", e),
                HostEvent::Debug(e) => debug!("Debug event: {}", e),
            }
        }
    }
}

#[derive(Clone, Debug)]
pub enum DebugArg {
    Str(&'static str),
    Val(RawVal),
}

impl From<RawVal> for DebugArg {
    fn from(rv: RawVal) -> Self {
        DebugArg::Val(rv)
    }
}

impl From<&'static str> for DebugArg {
    fn from(s: &'static str) -> Self {
        DebugArg::Str(s)
    }
}

impl Default for DebugArg {
    fn default() -> Self {
        DebugArg::Str("")
    }
}

impl Display for DebugArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DebugArg::Str(s) => write!(f, "{}", s),
            DebugArg::Val(rv) => write!(f, "{:?}", rv),
        }
    }
}

/// A cheap record type to store in the events buffer for diagnostic reporting
/// when something goes wrong. Should cost very little even when enabled. See
/// [host::Host::debug_event](crate::host::Host::debug_event) for normal use.
#[derive(Clone, Debug)]
pub struct DebugEvent {
    pub msg: Option<&'static str>,
    pub args: TinyVec<[DebugArg; 2]>,
}

impl core::fmt::Display for DebugEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.msg {
            None => {
                for arg in self.args.iter() {
                    write!(f, "{}", arg)?;
                }
                Ok(())
            }
            Some(fmt) => {
                let args = dyn_fmt::Arguments::new(fmt, self.args.as_slice());
                write!(f, "{}", args)
            }
        }
    }
}

impl DebugEvent {
    pub fn new() -> Self {
        Self {
            msg: None,
            args: Default::default(),
        }
    }

    pub fn msg(mut self, msg: &'static str) -> Self {
        self.msg = Some(msg);
        self
    }

    pub fn arg<T: Into<DebugArg>>(mut self, arg: T) -> Self {
        self.args.push(arg.into());
        self
    }
}

/// Combines a [DebugEvent] with a [Status] that created it, typically
/// used as a transient type when recording a (possibly enriched)
/// debug event for a status and then converting the status to a
/// HostError. See [host::Host::err](crate::host::Host::err) for normal use.
#[derive(Clone, Debug)]
pub struct DebugError {
    pub event: DebugEvent,
    pub status: Status,
}

impl DebugError {
    pub fn new<T>(status: T) -> Self
    where
        Status: From<T>,
    {
        let status: Status = status.into();
        Self {
            event: DebugEvent::new().msg("status").arg::<RawVal>(status.into()),
            status,
        }
    }

    pub fn general() -> Self {
        Self::new(xdr::ScUnknownErrorCode::General)
    }

    pub fn msg(mut self, msg: &'static str) -> Self {
        self.event = self.event.msg(msg);
        self
    }

    pub fn arg<T: Into<DebugArg>>(mut self, arg: T) -> Self {
        self.event = self.event.arg(arg);
        self
    }
}

impl From<xdr::Error> for DebugError {
    fn from(err: xdr::Error) -> Self {
        let msg = match err {
            xdr::Error::Invalid => "XDR error: invalid",
            xdr::Error::LengthExceedsMax => "XDR error: length exceeds max",
            xdr::Error::LengthMismatch => "XDR error: length mismatch",
            xdr::Error::NonZeroPadding => "XDR error: nonzero padding",
            xdr::Error::Utf8Error(_) => "XDR error: UTF-8 error",
            xdr::Error::Io(_) => "XDR error: IO error",
        };
        Self::new(xdr::ScUnknownErrorCode::Xdr).msg(msg)
    }
}

#[cfg(feature = "vm")]
impl From<wasmi::Error> for DebugError {
    fn from(err: wasmi::Error) -> Self {
        // At the moment we have a status code for each of the wasmi error types,
        // but we mighit reduce this to something coarser in the future, split
        // the name-reporting out from the code we return
        //
        // The errors from wasmi actually have much _more_ content (in the form
        // of Strings) that we're already eliding at this level, that we might
        // want to report for diagnostic purposes if we ever get dynamic strings
        // in the diagnostic buffer.
        use wasmi::Error::*;
        use wasmi::TrapCode::*;
        let code = match err {
            Validation(_) => ScVmErrorCode::Validation,
            Instantiation(_) => ScVmErrorCode::Instantiation,
            Function(_) => ScVmErrorCode::Function,
            Table(_) => ScVmErrorCode::Table,
            Memory(_) => ScVmErrorCode::Memory,
            Global(_) => ScVmErrorCode::Global,
            Value(_) => ScVmErrorCode::Value,
            Trap(wasmi::Trap::Host(err)) => {
                let status: Status = match err.downcast_ref::<HostError>() {
                    Some(he) => he.status,
                    None => ScUnknownErrorCode::General.into(),
                };
                return DebugError::new(status).msg("VM trapped with from host error");
            }
            Trap(wasmi::Trap::Code(c)) => match c {
                Unreachable => ScVmErrorCode::TrapUnreachable,
                MemoryAccessOutOfBounds => ScVmErrorCode::TrapMemoryAccessOutOfBounds,
                TableAccessOutOfBounds => ScVmErrorCode::TrapTableAccessOutOfBounds,
                ElemUninitialized => ScVmErrorCode::TrapElemUninitialized,
                DivisionByZero => ScVmErrorCode::TrapDivisionByZero,
                IntegerOverflow => ScVmErrorCode::TrapIntegerOverflow,
                InvalidConversionToInt => ScVmErrorCode::TrapInvalidConversionToInt,
                StackOverflow => ScVmErrorCode::TrapStackOverflow,
                UnexpectedSignature => ScVmErrorCode::TrapUnexpectedSignature,
                MemLimitExceeded => ScVmErrorCode::TrapMemLimitExceeded,
                CpuLimitExceeded => ScVmErrorCode::TrapCpuLimitExceeded,
            },
            Host(err) => {
                let status: Status = match err.downcast_ref::<HostError>() {
                    Some(he) => he.status,
                    None => ScUnknownErrorCode::General.into(),
                };
                return DebugError::new(status).msg("VM returned host error");
            }
        };
        Self::new(code).msg(code.name())
    }
}

#[cfg(feature = "vm")]
impl From<parity_wasm::elements::Error> for DebugError {
    fn from(err: parity_wasm::elements::Error) -> Self {
        use parity_wasm::SerializationError::*;
        let msg = match err {
            UnexpectedEof => "WASM deserialization error: unexpected EOF",
            InvalidMagic => "WASM deserialization error: invalid magic number",
            UnsupportedVersion(_) => "WASM deserialization error: unsupported version",
            InconsistentLength { .. } => "WASM deserialization error: inconsistent length",
            Other(_) => "WASM deserialization error: other",
            HeapOther(_) => "WASM deserialization error: heap, other",
            UnknownValueType(_) => "WASM deserialization error: unknown value type",
            UnknownTableElementType(_) => "WASM deserialization error: unknown table element type",
            NonUtf8String => "WASM deserialization error: non-UTF-8 string",
            UnknownExternalKind(_) => "WASM deserialization error: unknown external kind",
            UnknownInternalKind(_) => "WASM deserialization error: unknown internal kind",
            UnknownOpcode(_) => "WASM deserialization error: unknown opcode",
            InvalidVarUint1(_) => "WASM deserialization error: not an unsigned 1-bit number",
            InvalidVarInt32 => "WASM deserialization error: not a signed 32-bit number",
            InvalidVarInt64 => "WASM deserialization error: not a signed 64-bit number",
            InvalidVarUint32 => "WASM deserialization error: not an unsigned 32-bit number",
            InvalidVarUint64 => "WASM deserialization error: not an unsigned 64-bit number",
            InconsistentMetadata => "WASM deserialization error: inconsistent metadata",
            InvalidSectionId(_) => "WASM deserialization error: invalid section ID",
            SectionsOutOfOrder => "WASM deserialization error: sections out of order",
            DuplicatedSections(_) => "WASM deserialization error: duplicated sections",
            InvalidMemoryReference(_) => "WASM deserialization error: invalid memory reference",
            InvalidTableReference(_) => "WASM deserialization error: invalid table reference",
            InvalidLimitsFlags(_) => "WASM deserialization error: invalid limits flags",
            UnknownFunctionForm(_) => "WASM deserialization error: unknown function form",
            InvalidVarInt7(_) => "WASM deserialization error: not a signed 7-bit number",
            InconsistentCode => "WASM deserialization error: inconsistent code",
            InvalidSegmentFlags(_) => "WASM deserialization error: invalid segment flags",
            TooManyLocals => "WASM deserialization error: too many locals",
            DuplicatedNameSubsections(_) => {
                "WASM deserialization error: duplicated name subsections"
            }
            UnknownNameSubsectionType(_) => {
                "WASM deserialization error: unknown name subsection type"
            }
        };
        let code = ScVmErrorCode::Unknown;
        Self::new(code).msg(msg)
    }
}
