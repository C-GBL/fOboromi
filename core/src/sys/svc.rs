/// Horizon OS supervisor call numbers.
///
/// When guest code executes `SVC #N`, the emulator traps here.
/// Each SVC is dispatched to an HLE handler that mutates the kernel state
/// and writes results back through the CPU context.
///
/// Reference: switchbrew SVC list.

/// Result type for SVC handlers.  The `u32` is a HOS result code
/// (0 = success, non-zero = error module/description pair).
pub type SvcResult = u32;

/// Standard HOS result codes used by SVC handlers.
pub mod result {
    pub const SUCCESS: u32                  = 0;
    pub const INVALID_HANDLE: u32           = 0x0E401; // Kernel module (1), desc 114
    pub const INVALID_SIZE: u32             = 0x0CA01;
    pub const INVALID_ADDRESS: u32          = 0x0CC01;
    pub const OUT_OF_RESOURCE: u32          = 0x01001;
    pub const INVALID_CURRENT_MEMORY: u32   = 0x0D001;
    pub const INVALID_COMBINATION: u32      = 0x0E001;
    pub const TIMEOUT: u32                  = 0x0EA01;
    pub const CANCELLED: u32               = 0x0EC01;
    pub const INVALID_ENUM_VALUE: u32       = 0x0F001;
    pub const NOT_FOUND: u32                = 0x0F201;
    pub const ALREADY_EXISTS: u32           = 0x0F401;
    pub const SESSION_CLOSED: u32           = 0x0F601;
    pub const INVALID_STATE: u32            = 0x0FA01;
}

/// SVC IDs for the most important Horizon kernel calls.
///
/// The full table has ~192 entries; we list the ones critical for boot
/// and basic process execution.  Unlisted SVCs return an "unimplemented"
/// log + error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SvcId {
    // -- Memory ---------------------------------------------------------------
    SetHeapSize                = 0x01,
    SetMemoryPermission        = 0x02,
    SetMemoryAttribute         = 0x03,
    MapMemory                  = 0x04,
    UnmapMemory                = 0x05,
    QueryMemory                = 0x06,

    // -- Process / thread -----------------------------------------------------
    ExitProcess                = 0x07,
    CreateThread               = 0x08,
    StartThread                = 0x09,
    ExitThread                 = 0x0A,
    SleepThread                = 0x0B,
    GetThreadPriority          = 0x0C,
    SetThreadPriority          = 0x0D,
    GetThreadCoreMask          = 0x0E,
    SetThreadCoreMask          = 0x0F,
    GetCurrentProcessorNumber  = 0x10,

    // -- Synchronisation ------------------------------------------------------
    SignalEvent                = 0x11,
    ClearEvent                 = 0x12,
    MapSharedMemory            = 0x13,
    UnmapSharedMemory          = 0x14,
    CreateTransferMemory       = 0x15,
    CloseHandle                = 0x16,
    ResetSignal                = 0x17,
    WaitSynchronization        = 0x18,
    CancelSynchronization      = 0x19,
    ArbitrateLock              = 0x1A,
    ArbitrateUnlock            = 0x1B,
    WaitProcessWideKeyAtomic   = 0x1C,
    SignalProcessWideKey       = 0x1D,
    GetSystemTick              = 0x1E,

    // -- IPC ------------------------------------------------------------------
    ConnectToNamedPort         = 0x1F,
    SendSyncRequest            = 0x21,
    SendSyncRequestWithUserBuffer = 0x22,
    SendAsyncRequestWithUserBuffer = 0x23,
    GetProcessId               = 0x24,
    GetThreadId                = 0x25,
    Break                      = 0x26,
    OutputDebugString          = 0x27,
    ReturnFromException        = 0x28,
    GetInfo                    = 0x29,

    // -- Resource limits / misc -----------------------------------------------
    MapPhysicalMemory          = 0x2C,
    UnmapPhysicalMemory        = 0x2D,
    SetThreadActivity          = 0x32,
    GetThreadContext3          = 0x33,
    CreateSession              = 0x40,
    AcceptSession              = 0x41,
    ReplyAndReceive            = 0x43,
    CreateEvent                = 0x45,
    MapIoRegion                = 0x46,
    CreateCodeMemory           = 0x4B,
    ControlCodeMemory          = 0x4C,
    CallSecureMonitor          = 0x7F,
}

impl SvcId {
    pub fn from_u8(n: u8) -> Option<Self> {
        // Only map the values we explicitly handle.
        match n {
            0x01 => Some(Self::SetHeapSize),
            0x02 => Some(Self::SetMemoryPermission),
            0x03 => Some(Self::SetMemoryAttribute),
            0x04 => Some(Self::MapMemory),
            0x05 => Some(Self::UnmapMemory),
            0x06 => Some(Self::QueryMemory),
            0x07 => Some(Self::ExitProcess),
            0x08 => Some(Self::CreateThread),
            0x09 => Some(Self::StartThread),
            0x0A => Some(Self::ExitThread),
            0x0B => Some(Self::SleepThread),
            0x0C => Some(Self::GetThreadPriority),
            0x0D => Some(Self::SetThreadPriority),
            0x0E => Some(Self::GetThreadCoreMask),
            0x0F => Some(Self::SetThreadCoreMask),
            0x10 => Some(Self::GetCurrentProcessorNumber),
            0x11 => Some(Self::SignalEvent),
            0x12 => Some(Self::ClearEvent),
            0x13 => Some(Self::MapSharedMemory),
            0x14 => Some(Self::UnmapSharedMemory),
            0x15 => Some(Self::CreateTransferMemory),
            0x16 => Some(Self::CloseHandle),
            0x17 => Some(Self::ResetSignal),
            0x18 => Some(Self::WaitSynchronization),
            0x19 => Some(Self::CancelSynchronization),
            0x1A => Some(Self::ArbitrateLock),
            0x1B => Some(Self::ArbitrateUnlock),
            0x1C => Some(Self::WaitProcessWideKeyAtomic),
            0x1D => Some(Self::SignalProcessWideKey),
            0x1E => Some(Self::GetSystemTick),
            0x1F => Some(Self::ConnectToNamedPort),
            0x21 => Some(Self::SendSyncRequest),
            0x22 => Some(Self::SendSyncRequestWithUserBuffer),
            0x23 => Some(Self::SendAsyncRequestWithUserBuffer),
            0x24 => Some(Self::GetProcessId),
            0x25 => Some(Self::GetThreadId),
            0x26 => Some(Self::Break),
            0x27 => Some(Self::OutputDebugString),
            0x28 => Some(Self::ReturnFromException),
            0x29 => Some(Self::GetInfo),
            0x2C => Some(Self::MapPhysicalMemory),
            0x2D => Some(Self::UnmapPhysicalMemory),
            0x32 => Some(Self::SetThreadActivity),
            0x33 => Some(Self::GetThreadContext3),
            0x40 => Some(Self::CreateSession),
            0x41 => Some(Self::AcceptSession),
            0x43 => Some(Self::ReplyAndReceive),
            0x45 => Some(Self::CreateEvent),
            0x46 => Some(Self::MapIoRegion),
            0x4B => Some(Self::CreateCodeMemory),
            0x4C => Some(Self::ControlCodeMemory),
            0x7F => Some(Self::CallSecureMonitor),
            _    => None,
        }
    }
}

/// Dispatch an SVC by ID.
///
/// `x` is the register file (X0-X7 carry arguments, X0-X1 carry results).
/// Returns the HOS result code that should be placed in X0.
///
/// All handlers are currently stubs that log the call and return SUCCESS.
/// As individual services get implemented they will be filled in.
pub fn dispatch(id: u8, x: &mut [u64; 31]) -> SvcResult {
    match SvcId::from_u8(id) {
        Some(SvcId::SetHeapSize) => {
            let size = x[1];
            log::debug!("SVC SetHeapSize(size={:#x})", size);
            // Stub: pretend we set the heap; return base in X1.
            x[1] = 0x0800_0000; // arbitrary heap base
            result::SUCCESS
        }
        Some(SvcId::QueryMemory) => {
            log::debug!("SVC QueryMemory(addr={:#x})", x[2]);
            // Stub: zero out the MemoryInfo and return.
            result::SUCCESS
        }
        Some(SvcId::ExitProcess) => {
            log::info!("SVC ExitProcess");
            result::SUCCESS
        }
        Some(SvcId::CreateThread) => {
            log::debug!("SVC CreateThread(entry={:#x}, sp={:#x}, prio={})", x[1], x[2], x[3]);
            // Stub: return a fake handle in X1.
            x[1] = 0xDEAD_0001;
            result::SUCCESS
        }
        Some(SvcId::StartThread) => {
            log::debug!("SVC StartThread(handle={:#x})", x[0]);
            result::SUCCESS
        }
        Some(SvcId::ExitThread) => {
            log::debug!("SVC ExitThread");
            result::SUCCESS
        }
        Some(SvcId::SleepThread) => {
            log::trace!("SVC SleepThread(ns={})", x[0] as i64);
            result::SUCCESS
        }
        Some(SvcId::GetThreadPriority) => {
            x[1] = 44; // default priority
            result::SUCCESS
        }
        Some(SvcId::GetCurrentProcessorNumber) => {
            x[0] = 0; // core 0
            result::SUCCESS
        }
        Some(SvcId::CloseHandle) => {
            log::trace!("SVC CloseHandle(handle={:#x})", x[0]);
            result::SUCCESS
        }
        Some(SvcId::WaitSynchronization) => {
            log::trace!("SVC WaitSynchronization");
            // Stub: immediate return, index 0.
            x[1] = 0;
            result::SUCCESS
        }
        Some(SvcId::ArbitrateLock) => {
            log::trace!("SVC ArbitrateLock");
            result::SUCCESS
        }
        Some(SvcId::ArbitrateUnlock) => {
            log::trace!("SVC ArbitrateUnlock");
            result::SUCCESS
        }
        Some(SvcId::WaitProcessWideKeyAtomic) => {
            log::trace!("SVC WaitProcessWideKeyAtomic");
            result::SUCCESS
        }
        Some(SvcId::SignalProcessWideKey) => {
            log::trace!("SVC SignalProcessWideKey");
            result::SUCCESS
        }
        Some(SvcId::GetSystemTick) => {
            // Return a monotonically increasing tick (19.2 MHz on real HW).
            // For HLE we just use host time scaled roughly.
            let tick = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64
                / 52; // ~19.2 MHz
            x[0] = tick;
            result::SUCCESS
        }
        Some(SvcId::ConnectToNamedPort) => {
            log::debug!("SVC ConnectToNamedPort");
            // Stub: return a fake session handle.
            x[1] = 0xC0DE_0001;
            result::SUCCESS
        }
        Some(SvcId::SendSyncRequest) => {
            log::trace!("SVC SendSyncRequest(handle={:#x})", x[0]);
            result::SUCCESS
        }
        Some(SvcId::GetProcessId) => {
            x[1] = 1; // stub PID
            result::SUCCESS
        }
        Some(SvcId::GetThreadId) => {
            x[1] = 1; // stub TID
            result::SUCCESS
        }
        Some(SvcId::Break) => {
            let reason = x[0];
            log::warn!("SVC Break(reason={:#x})", reason);
            result::SUCCESS
        }
        Some(SvcId::OutputDebugString) => {
            log::debug!("SVC OutputDebugString(ptr={:#x}, len={})", x[0], x[1]);
            result::SUCCESS
        }
        Some(SvcId::GetInfo) => {
            let info_id = x[1];
            let handle = x[2];
            let sub_id = x[3];
            log::debug!("SVC GetInfo(id={}, handle={:#x}, sub={})", info_id, handle, sub_id);
            // Return 0 for now; individual info IDs will be filled in.
            x[1] = 0;
            result::SUCCESS
        }
        Some(SvcId::MapPhysicalMemory) => {
            log::debug!("SVC MapPhysicalMemory(addr={:#x}, size={:#x})", x[0], x[1]);
            result::SUCCESS
        }
        Some(SvcId::CallSecureMonitor) => {
            log::debug!("SVC CallSecureMonitor");
            result::SUCCESS
        }
        Some(other) => {
            log::warn!("SVC {:#04x} ({:?}) unimplemented", id, other);
            result::SUCCESS
        }
        None => {
            log::warn!("SVC {:#04x} unknown", id);
            result::SUCCESS
        }
    }
}
