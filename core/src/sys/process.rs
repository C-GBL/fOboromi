use std::collections::HashMap;

/// Unique identifier for a kernel process.
pub type ProcessId = u64;

/// Unique identifier for a thread within a process.
pub type ThreadId = u64;

/// Current execution state of a thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadState {
    Created,
    Ready,
    Running,
    Waiting,
    Terminated,
}

/// Saved CPU context for a thread (ARMv8 AArch64 registers).
#[derive(Debug, Clone)]
pub struct CpuContext {
    /// General-purpose registers X0-X30.
    pub x: [u64; 31],
    /// Stack pointer.
    pub sp: u64,
    /// Program counter.
    pub pc: u64,
    /// Process state (PSTATE/NZCV flags).
    pub pstate: u32,
    /// TPIDR_EL0 — thread-local storage pointer.
    pub tpidr_el0: u64,
}

impl Default for CpuContext {
    fn default() -> Self {
        Self {
            x: [0; 31],
            sp: 0,
            pc: 0,
            pstate: 0,
            tpidr_el0: 0,
        }
    }
}

/// A single thread of execution inside a process.
pub struct Thread {
    pub id: ThreadId,
    pub name: String,
    pub state: ThreadState,
    pub context: CpuContext,
    /// Which CPU core this thread is pinned to (-1 = any).
    pub ideal_core: i32,
    pub priority: u32,
    /// The process that owns this thread.
    pub owner_pid: ProcessId,
}

impl Thread {
    pub fn new(id: ThreadId, owner_pid: ProcessId, entry: u64, stack_top: u64) -> Self {
        let mut ctx = CpuContext::default();
        ctx.pc = entry;
        ctx.sp = stack_top;
        Self {
            id,
            name: String::new(),
            state: ThreadState::Created,
            context: ctx,
            ideal_core: -1,
            priority: 44, // default HOS priority
            owner_pid,
        }
    }
}

/// Memory mapping within a process address space.
#[derive(Debug, Clone)]
pub struct MemoryRegion {
    pub base: u64,
    pub size: u64,
    pub perm: MemoryPermission,
    pub state: MemoryState,
}

/// Permission bits for a memory region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryPermission(pub u32);

impl MemoryPermission {
    pub const NONE: Self = Self(0);
    pub const READ: Self = Self(1);
    pub const WRITE: Self = Self(2);
    pub const EXEC: Self = Self(4);
    pub const RW: Self = Self(1 | 2);
    pub const RX: Self = Self(1 | 4);
    pub const RWX: Self = Self(1 | 2 | 4);
}

/// Type tag for a memory region (mirrors HOS memory types).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryState {
    Free,
    CodeStatic,
    CodeMutable,
    Heap,
    SharedMemory,
    MappedMemory,
    Stack,
    ThreadLocal,
}

/// A kernel-managed process.
pub struct Process {
    pub pid: ProcessId,
    pub name: String,
    /// Threads owned by this process.
    pub threads: HashMap<ThreadId, Thread>,
    /// Virtual memory regions.
    pub memory_map: Vec<MemoryRegion>,
    /// Base address where the process code was loaded.
    pub code_base: u64,
    /// Size of the code region.
    pub code_size: u64,
    /// Heap region base.
    pub heap_base: u64,
    /// Current heap size.
    pub heap_size: u64,
    /// Stack region base for the main thread.
    pub stack_base: u64,
    /// Stack size.
    pub stack_size: u64,
    /// Whether this is a kernel-internal (HLE) process.
    pub is_hle: bool,
    /// Next thread ID to allocate.
    next_thread_id: ThreadId,
}

impl Process {
    pub fn new(pid: ProcessId, name: &str, code_base: u64, code_size: u64) -> Self {
        Self {
            pid,
            name: name.to_string(),
            threads: HashMap::new(),
            memory_map: Vec::new(),
            code_base,
            code_size,
            heap_base: 0,
            heap_size: 0,
            stack_base: 0,
            stack_size: 0,
            is_hle: false,
            next_thread_id: 1,
        }
    }

    /// Create the main thread for this process.
    pub fn create_main_thread(&mut self, entry: u64, stack_top: u64) -> ThreadId {
        let tid = self.next_thread_id;
        self.next_thread_id += 1;
        let mut thread = Thread::new(tid, self.pid, entry, stack_top);
        thread.name = format!("{}-main", self.name);
        thread.state = ThreadState::Ready;
        self.threads.insert(tid, thread);
        tid
    }

    /// Create an additional thread.
    pub fn create_thread(&mut self, entry: u64, stack_top: u64, priority: u32) -> ThreadId {
        let tid = self.next_thread_id;
        self.next_thread_id += 1;
        let mut thread = Thread::new(tid, self.pid, entry, stack_top);
        thread.priority = priority;
        thread.state = ThreadState::Ready;
        self.threads.insert(tid, thread);
        tid
    }

    /// Add a memory mapping.
    pub fn map_memory(&mut self, base: u64, size: u64, perm: MemoryPermission, state: MemoryState) {
        self.memory_map.push(MemoryRegion {
            base,
            size,
            perm,
            state,
        });
    }
}
