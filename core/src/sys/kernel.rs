use std::collections::HashMap;

use crate::cpu::cpu_manager::CpuManager;
use crate::sys::process::{
    MemoryPermission, MemoryState, Process, ProcessId, ThreadId, ThreadState,
};
use crate::sys::svc;

/// Handle value used by guest code to reference kernel objects.
pub type Handle = u32;

/// The HLE Horizon kernel.
///
/// Manages processes, threads, handles, and IPC.  SVC traps from guest
/// code are dispatched through [`svc::dispatch`] and mutate state here.
pub struct Kernel {
    /// All live processes keyed by PID.
    pub processes: HashMap<ProcessId, Process>,
    /// CPU manager (8 cores + 12 GB shared RAM).
    pub cpu: CpuManager,
    /// Monotonically increasing PID counter.
    next_pid: ProcessId,
    /// Handle-to-(pid, tid) mapping for thread handles.
    pub handle_table: HashMap<Handle, (ProcessId, ThreadId)>,
    /// Next handle value to allocate.
    next_handle: Handle,
    /// Named service ports registered by sysmodules (name -> handle).
    pub named_ports: HashMap<String, Handle>,
    /// Which (pid, tid) is currently executing on each core (indexed by core id).
    pub running_on: [Option<(ProcessId, ThreadId)>; 8],
    /// Boot has completed — services are up, home menu can launch.
    pub booted: bool,
}

impl Kernel {
    pub fn new() -> Self {
        Self {
            processes: HashMap::new(),
            cpu: CpuManager::new(),
            next_pid: 1,
            handle_table: HashMap::new(),
            next_handle: 0x1000,
            named_ports: HashMap::new(),
            running_on: [None; 8],
            booted: false,
        }
    }

    // -- Process management ---------------------------------------------------

    /// Create a new process and return its PID.
    pub fn create_process(&mut self, name: &str, code_base: u64, code_size: u64) -> ProcessId {
        let pid = self.next_pid;
        self.next_pid += 1;
        let process = Process::new(pid, name, code_base, code_size);
        log::info!("kernel: created process '{}' pid={}", name, pid);
        self.processes.insert(pid, process);
        pid
    }

    /// Create the main thread for a process and return a (handle, tid) pair.
    pub fn create_main_thread(
        &mut self,
        pid: ProcessId,
        entry: u64,
        stack_top: u64,
    ) -> Option<(Handle, ThreadId)> {
        // Scope the mutable borrow of processes so it's released before alloc_handle.
        let (tid, name) = {
            let process = self.processes.get_mut(&pid)?;
            let tid = process.create_main_thread(entry, stack_top);
            (tid, process.name.clone())
        };
        let handle = self.alloc_handle(pid, tid);
        log::info!("kernel: main thread for '{}' tid={} entry={:#x}", name, tid, entry);
        Some((handle, tid))
    }

    /// Map a memory region into a process's virtual address space.
    pub fn map_process_memory(
        &mut self,
        pid: ProcessId,
        base: u64,
        size: u64,
        perm: MemoryPermission,
        state: MemoryState,
    ) {
        if let Some(process) = self.processes.get_mut(&pid) {
            process.map_memory(base, size, perm, state);
        }
    }

    // -- Handle management ----------------------------------------------------

    fn alloc_handle(&mut self, pid: ProcessId, tid: ThreadId) -> Handle {
        let h = self.next_handle;
        self.next_handle += 1;
        self.handle_table.insert(h, (pid, tid));
        h
    }

    // -- Named port (IPC service discovery) -----------------------------------

    /// Register a named service port (called by HLE sysmodules during boot).
    pub fn register_port(&mut self, name: &str, handle: Handle) {
        log::debug!("kernel: registered port '{}'", name);
        self.named_ports.insert(name.to_string(), handle);
    }

    /// Connect to a named port (SVC ConnectToNamedPort).
    pub fn connect_to_port(&self, name: &str) -> Option<Handle> {
        self.named_ports.get(name).copied()
    }

    // -- Scheduling (round-robin stub) ----------------------------------------

    /// Schedule and execute one timeslice across all cores.
    ///
    /// This is a minimal round-robin scheduler: it collects all Ready
    /// threads, assigns them to cores, runs one step on each core, then
    /// saves context back.
    pub fn step(&mut self) {
        // Collect ready threads.
        let mut ready: Vec<(ProcessId, ThreadId)> = Vec::new();
        for process in self.processes.values() {
            for thread in process.threads.values() {
                if thread.state == ThreadState::Ready || thread.state == ThreadState::Running {
                    ready.push((process.pid, thread.id));
                }
            }
        }

        // Assign to cores (up to 8).
        for (core_id, assignment) in ready.iter().enumerate().take(8) {
            let (pid, tid) = *assignment;
            self.running_on[core_id] = Some((pid, tid));

            if let Some(process) = self.processes.get(&pid) {
                if let Some(thread) = process.threads.get(&tid) {
                    // Load context into CPU core.
                    let core = &self.cpu.cores[core_id];
                    core.set_pc(thread.context.pc);
                    core.set_sp(thread.context.sp);
                    for r in 0..31u32 {
                        core.set_x(r, thread.context.x[r as usize]);
                    }
                }
            }
        }

        // Execute one step on each active core.
        for (core_id, slot) in self.running_on.iter().enumerate() {
            if slot.is_some() {
                self.cpu.cores[core_id].step();
            }
        }

        // Save context back.
        for (core_id, slot) in self.running_on.iter().enumerate() {
            if let Some((pid, tid)) = slot {
                if let Some(process) = self.processes.get_mut(pid) {
                    if let Some(thread) = process.threads.get_mut(tid) {
                        let core = &self.cpu.cores[core_id];
                        thread.context.pc = core.get_pc();
                        thread.context.sp = core.get_sp();
                        for r in 0..31u32 {
                            thread.context.x[r as usize] = core.get_x(r);
                        }
                        thread.state = ThreadState::Running;
                    }
                }
            }
        }
    }

    // -- SVC dispatch ---------------------------------------------------------

    /// Handle an SVC trap from the given core.
    ///
    /// Reads the SVC number and register state from the core, dispatches
    /// via [`svc::dispatch`], and writes the result back.
    pub fn handle_svc(&mut self, core_id: usize, svc_num: u8) {
        let core = &self.cpu.cores[core_id];

        let mut regs = [0u64; 31];
        for r in 0..31u32 {
            regs[r as usize] = core.get_x(r);
        }

        let result = svc::dispatch(svc_num, &mut regs);
        regs[0] = result as u64;

        let core = &self.cpu.cores[core_id];
        for r in 0..31u32 {
            core.set_x(r, regs[r as usize]);
        }
    }

    // -- Load code into memory ------------------------------------------------

    /// Load raw binary code into the shared memory of the CPU manager at the
    /// given virtual address.  Returns the number of bytes written.
    pub fn load_code(&self, vaddr: u64, code: &[u8]) -> usize {
        // Write through core 0 (all cores share the same backing memory).
        let core = &self.cpu.cores[0];
        for (i, chunk) in code.chunks(4).enumerate() {
            if chunk.len() == 4 {
                let word = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                core.write_u32(vaddr + (i as u64) * 4, word);
            } else {
                // Trailing bytes that don't fill a word.
                for (j, &byte) in chunk.iter().enumerate() {
                    let offset = vaddr + (i as u64) * 4 + j as u64;
                    // Write byte-by-byte via a read-modify-write of the word.
                    let aligned = offset & !3;
                    let shift = (offset & 3) * 8;
                    let mut word = core.read_u32(aligned);
                    word &= !(0xFF << shift);
                    word |= (byte as u32) << shift;
                    core.write_u32(aligned, word);
                }
            }
        }
        code.len()
    }
}
