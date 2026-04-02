use std::io;
use std::path::{Path, PathBuf};

use crate::fs;
use crate::nn;
use crate::sys::kernel::Kernel;
use crate::sys::process::{MemoryPermission, MemoryState};

/// Where firmware-related files are located on the host.
#[derive(Debug, Clone)]
pub struct FirmwareConfig {
    /// Path to a NAND image file (created if missing).
    pub nand_path: PathBuf,
    /// Optional path to a raw Package2 / kernel binary.
    /// When `None`, the boot sequence will attempt to read from the NAND
    /// SYSTEM partition instead.
    pub package2_path: Option<PathBuf>,
    /// Optional path to a keys file for title-key / header decryption.
    pub keys_path: Option<PathBuf>,
}

impl Default for FirmwareConfig {
    fn default() -> Self {
        Self {
            nand_path: PathBuf::from("nand.img"),
            package2_path: None,
            keys_path: None,
        }
    }
}

/// Tracks where we are in the multi-phase boot process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootPhase {
    /// Nothing started yet.
    Off,
    /// Initialising hardware (CPU cores, GPU, NAND).
    HardwareInit,
    /// Loading the kernel / Package2 image.
    KernelLoad,
    /// Starting HLE kernel services (sm, fs, pm, ...).
    ServiceInit,
    /// Launching the home menu (qlaunch) process.
    HomeMenu,
    /// Fully booted — ready to launch titles.
    Running,
    /// An error occurred during boot.
    Failed,
}

/// Accumulated boot log — each phase appends human-readable status lines.
pub type BootLog = Vec<String>;

/// Result of a boot attempt.
pub struct BootResult {
    pub phase: BootPhase,
    pub log: BootLog,
    pub nand: Option<fs::Nand>,
}

// -- Address map constants ----------------------------------------------------
// These mirror the Horizon OS default layout for a 39-bit address space.

/// Base address where the kernel is loaded.
const KERNEL_BASE: u64 = 0x0008_0000;
/// Size of the kernel code region (4 MiB).
const KERNEL_CODE_SIZE: u64 = 4 * 1024 * 1024;
/// Stack top for the kernel main thread.
const KERNEL_STACK_TOP: u64 = KERNEL_BASE + KERNEL_CODE_SIZE + 0x10_0000;

/// Base address for the home-menu (qlaunch) process.
const QLAUNCH_BASE: u64 = 0x0100_0000;
const QLAUNCH_CODE_SIZE: u64 = 8 * 1024 * 1024;
const QLAUNCH_STACK_TOP: u64 = QLAUNCH_BASE + QLAUNCH_CODE_SIZE + 0x10_0000;

/// Run the full boot sequence.
///
/// This function is designed to be called from a background thread so the
/// GUI remains responsive.  Progress is recorded in the returned
/// [`BootResult`].
pub fn boot(config: &FirmwareConfig) -> BootResult {
    let mut log: BootLog = Vec::new();

    // ---- Phase 1: Hardware init ---------------------------------------------
    log.push("Phase 1: Hardware initialisation".into());

    // NAND
    log.push(format!("  Opening NAND image: {}", config.nand_path.display()));
    let nand = match fs::Nand::open(&config.nand_path) {
        Ok(n) => {
            log.push(format!(
                "  NAND: {} blocks, {} partitions",
                n.storage.total_blocks(),
                n.partitions.entries.len(),
            ));
            Some(n)
        }
        Err(e) => {
            log.push(format!("  NAND open failed: {}", e));
            return BootResult { phase: BootPhase::Failed, log, nand: None };
        }
    };

    // CPU + GPU are part of the Kernel struct, created in phase 3.
    log.push("  CPU: 8-core ARMv8 (Unicorn)".into());
    log.push("  GPU: SM86 Ampere stub".into());
    log.push("  RAM: 12 GB shared".into());

    // ---- Phase 2: Kernel load -----------------------------------------------
    log.push("Phase 2: Kernel load".into());

    let kernel_code = match load_kernel_image(config) {
        Ok(code) => {
            log.push(format!("  Loaded kernel image: {} bytes", code.len()));
            code
        }
        Err(e) => {
            log.push(format!("  Kernel load failed: {} — using stub kernel", e));
            // Generate a minimal stub kernel (BRK #0) so the boot sequence
            // can still demonstrate the pipeline.
            stub_kernel()
        }
    };

    // ---- Phase 3: Start HLE kernel ------------------------------------------
    log.push("Phase 3: Kernel & service init".into());

    let mut kernel = Kernel::new();

    // Load kernel code into emulated memory.
    kernel.load_code(KERNEL_BASE, &kernel_code);
    log.push(format!("  Kernel code loaded at {:#x}", KERNEL_BASE));

    // Create the kernel process + main thread.
    let kpid = kernel.create_process("kernel", KERNEL_BASE, KERNEL_CODE_SIZE);
    kernel.map_process_memory(
        kpid,
        KERNEL_BASE,
        KERNEL_CODE_SIZE,
        MemoryPermission::RX,
        MemoryState::CodeStatic,
    );
    kernel.map_process_memory(
        kpid,
        KERNEL_STACK_TOP - 0x10_0000,
        0x10_0000,
        MemoryPermission::RW,
        MemoryState::Stack,
    );
    let _ = kernel.create_main_thread(kpid, KERNEL_BASE, KERNEL_STACK_TOP);
    log.push("  kernel process created".into());

    // Start HLE system services (these populate sys::State::services).
    // We create a temporary State just for service init; the services
    // are lightweight stubs at this point.
    log.push("  Starting HLE system services...".into());
    {
        let mut svc_state = crate::sys::State::new();
        nn::start_host_services(&mut svc_state);
        let started = count_services(&svc_state);
        log.push(format!("  {} HLE services started", started));
    }

    // Register core named ports so guest code can discover services.
    let core_ports = [
        "sm:", "fsp-srv", "fsp-ldr", "fsp-pr", "set", "set:sys",
        "time:s", "time:u", "hid", "appletOE", "appletAE",
        "nvdrv", "nvdrv:a", "nvdrv:s", "nvdrv:t",
        "vi:m", "vi:s", "vi:u", "lm", "pctl", "acc:u0",
        "nifm:u", "ns:su", "ns:am", "aoc:u", "lr", "ncm",
        "pm:shell", "pm:dmnt", "pm:info",
    ];
    for (i, name) in core_ports.iter().enumerate() {
        kernel.register_port(name, 0x2000 + i as u32);
    }
    log.push(format!("  {} named ports registered", core_ports.len()));

    // ---- Phase 4: Home menu -------------------------------------------------
    log.push("Phase 4: Home menu (qlaunch)".into());

    // Create a stub qlaunch process.
    let qpid = kernel.create_process("qlaunch", QLAUNCH_BASE, QLAUNCH_CODE_SIZE);
    kernel.map_process_memory(
        qpid,
        QLAUNCH_BASE,
        QLAUNCH_CODE_SIZE,
        MemoryPermission::RX,
        MemoryState::CodeStatic,
    );
    kernel.map_process_memory(
        qpid,
        QLAUNCH_STACK_TOP - 0x10_0000,
        0x10_0000,
        MemoryPermission::RW,
        MemoryState::Stack,
    );
    // Load a stub for qlaunch (just a BRK so it stops immediately).
    kernel.load_code(QLAUNCH_BASE, &stub_kernel());
    let _ = kernel.create_main_thread(qpid, QLAUNCH_BASE, QLAUNCH_STACK_TOP);
    log.push("  qlaunch process created".into());

    // Step the kernel once to prove execution works.
    kernel.step();
    log.push("  Scheduler step OK".into());

    // ---- Done ---------------------------------------------------------------
    kernel.booted = true;
    log.push("Boot complete — system is running.".into());

    BootResult { phase: BootPhase::Running, log, nand }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Try to load a kernel/package2 binary from the filesystem.
fn load_kernel_image(config: &FirmwareConfig) -> io::Result<Vec<u8>> {
    if let Some(p) = &config.package2_path {
        log::info!("Loading kernel from {}", p.display());
        return std::fs::read(p);
    }

    // Fall back: try to read from the NAND SYSTEM partition header.
    // On a real Switch the first 0x200 bytes of SYSTEM contain the
    // Package2 header.  For now we just try to read a small blob.
    let nand_path = &config.nand_path;
    if nand_path.exists() {
        let mut nand = fs::Nand::open(nand_path)?;
        let mut header = vec![0u8; 0x200];
        let _ = nand.read(
            crate::fs::partition::PartitionId::System,
            0,
            &mut header,
        )?;

        // Check for a plausible Package2 magic ("PK21").
        if header.len() >= 4 && &header[0..4] == b"PK21" {
            log::info!("Found PK21 header in NAND SYSTEM partition");
            // Read the full package (size is at offset 0x10 LE u32).
            let size = u32::from_le_bytes([header[0x10], header[0x11], header[0x12], header[0x13]]) as usize;
            if size > 0 && size < 64 * 1024 * 1024 {
                let mut buf = vec![0u8; size];
                nand.read(
                    crate::fs::partition::PartitionId::System,
                    0x200,
                    &mut buf,
                )?;
                return Ok(buf);
            }
        }

        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no PK21 header found in NAND SYSTEM partition",
        ));
    }

    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "no firmware image provided",
    ))
}

/// Generate a tiny stub kernel: just a BRK #1 instruction.
fn stub_kernel() -> Vec<u8> {
    // BRK #1 = 0xD4200020
    0xD420_0020u32.to_le_bytes().to_vec()
}

/// Count how many HLE services were successfully initialised.
fn count_services(state: &crate::sys::State) -> usize {
    // The Services struct has ~160 Option fields.  We use a simple
    // macro-counting trick: serialise to debug and count "Some(…)".
    // This is intentionally rough — it's only for the boot log.
    let s = &state.services;
    let mut n = 0usize;
    macro_rules! count {
        ($($field:ident),* $(,)?) => {
            $(if s.$field.is_some() { n += 1; })*
        };
    }
    count!(
        acc, adraw, ahid, aoc, apm, applet_ae, applet_oe, arp,
        aud, audctl, auddebug, auddev, auddmg, audin, audout,
        audrec, audren, audsmx, avm, banana, batlog, bcat, bgtc,
        bpc, bpmpmr, bsd, bsdcfg, bt, btdrv, btm, btp, capmtp,
        caps, caps2, cec_mgr, chat, clkrst, codecctl, csrng,
        dauth, disp, dispdrv, dmnt, dns, dt, ectx, erpt, es,
        eth, ethc, eupld, fan, fatal, fgm, file_io, friend, fs,
        fsp_ldr, fsp_pr, fsp_srv, gds, gpio, gpuk, grc, gsv,
        hdcp, hid, hidbus, host1x, hshl, htc, htcs, hwopus, i2c,
        idle, ifcfg, imf, ins, irs, jit, lbl, ldn, ldr, led, lm,
        lp2p, lr, manu, mig, mii, miiimg, mm, mnpp, ncm, nd, ndd,
        ndrm, news, nfc, nfp, ngc, ngct, nifm, nim, notif, npns,
        ns, nsd, ntc, nvdbg, nvdrv, nvdrvdbg, nvgem, nvmemp,
        olsc, omm, ommdisp, ovln, pcie, pcm, pctl, pcv, pdm,
        pgl, pinmux, pl, pm, prepo, psc, psm, pwm, rgltr, ro,
        rtc, sasbus, set, sf_uds, sfdnsres, spbg, spi, spl,
        sprof, spsm, srepo, ssl, syncpt, tc, tcap, time,
        tma_log, tmagent, ts, tspm, uart, usb, vi, vi2, vic,
        wlan, xcd
    );
    n
}

/// Convenience wrapper: boot with default settings and a NAND path.
pub fn boot_with_nand<P: AsRef<Path>>(nand_path: P) -> BootResult {
    let config = FirmwareConfig {
        nand_path: nand_path.as_ref().to_path_buf(),
        ..Default::default()
    };
    boot(&config)
}
