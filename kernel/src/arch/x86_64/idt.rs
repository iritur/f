// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Exceptions, and the dump you read when one happens.
//!
//! # What this replaces
//!
//! Nothing. Until now `IDTR` has held whatever the firmware left, so every
//! exception has been a triple fault: the processor tries to deliver, fails,
//! tries to deliver the failure, fails again, and resets. The two bugs found
//! while building the address space — a global offset table placed outside the
//! kernel window, and a lazily read memory map that stopped being mapped — both
//! presented as a machine that silently stopped. Finding either meant asking the
//! *emulator* what happened, which is a technique that does not survive contact
//! with real hardware.
//!
//! After this file, a fault prints what it was, where it was, and what the
//! processor was holding at the time.
//!
//! # Why hand-written stubs rather than the interrupt ABI
//!
//! Rust can generate an interrupt entry sequence directly, and it hands the
//! handler a neat structure containing the five words the processor pushed. It
//! does not hand over the general-purpose registers, because it saves and
//! restores them around the call — correctly, invisibly, and exactly where a
//! person reading a crash wants to look.
//!
//! So the stubs are assembly: push everything, pass a pointer to all of it, and
//! let the handler print registers that were live at the moment of the fault.
//! The milestone calls for "exception handlers with a register dump you will
//! read hundreds of times", and half a dump is not that.
//!
//! The same path serves the timer at M2: the frame is restored and `iretq`
//! returns, so a handler that chooses to continue can.

use core::arch::global_asm;
use core::fmt::Write;

use super::{apic, gdt};
// The macro is `#[macro_export]`ed, so it lives at the crate root. Importing it
// here is the opposite of the situation in `main.rs`, where the same import
// would be a redefinition — the root is where it already is.
use crate::kprintln;
use crate::percpu::PerCpu;

/// Vectors the processor defines. Everything above is available for devices.
const EXCEPTION_VECTORS: usize = 32;

/// Entries in the table. The full space, so an unexpected vector is a report
/// rather than a limit violation — which would itself be an exception.
const IDT_ENTRIES: usize = 256;

/// Present, ring 0, 64-bit interrupt gate.
///
/// An interrupt gate rather than a trap gate: it clears the interrupt flag on
/// entry, so a handler is not re-entered by the device that is still asserting.
const GATE_FLAGS: u8 = 0x8E;

/// The double fault, which is the one that needs its own stack.
const DOUBLE_FAULT: usize = 8;

/// The breakpoint, which is the one that is not a failure.
const BREAKPOINT: u64 = 3;

/// The page fault, which is the one with an informative error code.
const PAGE_FAULT: u64 = 14;

/// Every register the processor was holding, in the order the stubs push them.
///
/// `repr(C)` and the field order are load-bearing twice over: this is a view of
/// the stack, and the stubs restore from it. A field moved here without the same
/// move in the assembly is a crash in the code that reports crashes.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Frame {
    /// General-purpose registers. The stubs push them in reverse of this order,
    /// so the lowest address holds the last one pushed.
    pub r15: u64,
    /// See [`Frame::r15`].
    pub r14: u64,
    /// See [`Frame::r15`].
    pub r13: u64,
    /// See [`Frame::r15`].
    pub r12: u64,
    /// See [`Frame::r15`].
    pub r11: u64,
    /// See [`Frame::r15`].
    pub r10: u64,
    /// See [`Frame::r15`].
    pub r9: u64,
    /// See [`Frame::r15`].
    pub r8: u64,
    /// See [`Frame::r15`].
    pub rdi: u64,
    /// See [`Frame::r15`].
    pub rsi: u64,
    /// See [`Frame::r15`].
    pub rbp: u64,
    /// See [`Frame::r15`].
    pub rbx: u64,
    /// See [`Frame::r15`].
    pub rdx: u64,
    /// See [`Frame::r15`].
    pub rcx: u64,
    /// See [`Frame::r15`].
    pub rax: u64,
    /// Which exception. Pushed by the stub, because the processor does not say.
    pub vector: u64,
    /// The processor's error code, or zero where it does not produce one. The
    /// stub pushes a zero in that case, so every frame has the same shape.
    pub error: u64,
    /// Where the fault happened. Pushed by the processor.
    pub rip: u64,
    /// Code segment at the fault. Pushed by the processor.
    pub cs: u64,
    /// Flags at the fault. Pushed by the processor.
    pub rflags: u64,
    /// Stack pointer at the fault. Pushed by the processor.
    pub rsp: u64,
    /// Stack segment at the fault. Pushed by the processor.
    pub ss: u64,
}

/// One gate.
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct Gate {
    offset_low: u16,
    selector: u16,
    ist: u8,
    flags: u8,
    offset_mid: u16,
    offset_high: u32,
    _reserved: u32,
}

impl Gate {
    const EMPTY: Self = Self {
        offset_low: 0,
        selector: 0,
        ist: 0,
        flags: 0,
        offset_mid: 0,
        offset_high: 0,
        _reserved: 0,
    };

    fn new(handler: u64, ist: u8) -> Self {
        Self {
            offset_low: handler as u16,
            selector: gdt::KERNEL_CODE,
            ist,
            flags: GATE_FLAGS,
            offset_mid: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            _reserved: 0,
        }
    }
}

/// Per core, like every other table the kernel owns.
///
/// The contents are identical on every core today and will not always be —
/// interrupt routing is the first thing that differs once there is more than
/// one core to route to. Sharding it now costs `MAX_CPUS * 4 KiB` of `.bss`
/// and removes the question; sharing it would buy 28 KiB back and put a
/// read-mostly global in the one kernel that has decided not to have any.
///
/// *Reversal:* if that memory is ever worth the exception, the table is
/// genuinely write-once, and a shared immutable one is defensible — but it
/// needs a type that says "written once at boot, read by hardware thereafter",
/// not a `static mut`.
static IDT: PerCpu<[Gate; IDT_ENTRIES]> = PerCpu::new([Gate::EMPTY; IDT_ENTRIES]);

unsafe extern "C" {
    /// Addresses of the thirty-two exception stubs, built by the assembly below
    /// so that the list of them exists in exactly one place.
    static isr_table: [u64; EXCEPTION_VECTORS];

    /// The stub for the local APIC's timer.
    ///
    /// A label rather than a table entry, for the same reason as the spurious
    /// one below: it is a device vector, not the thirty-third exception.
    static isr_timer: u8;

    /// The stub for an interrupt the local APIC could not explain.
    ///
    /// A label rather than an entry in the table above, because it is not the
    /// thirty-third exception: it is a device vector, it lives at the far end
    /// of the table, and putting it in a list indexed by vector number would
    /// mean two hundred and twenty-four empty entries between.
    static isr_spurious: u8;
}

/// Install the interrupt descriptor table.
///
/// # Safety
///
/// Call once per core, on that core, after [`gdt::init`] on the same core,
/// because every gate names the kernel code selector that installs.
pub unsafe fn init() {
    let idt = IDT.mine().cast::<Gate>();
    let stubs = (&raw const isr_table).cast::<u64>();

    for vector in 0..EXCEPTION_VECTORS {
        // The double fault runs on its own stack, because the usual reason for
        // one is that the stack it would otherwise use is the problem.
        let ist = if vector == DOUBLE_FAULT { gdt::DOUBLE_FAULT_IST } else { 0 };

        let stub = stubs.wrapping_add(vector);
        // SAFETY: the table is thirty-two entries built by the assembly below,
        // and `vector` is an index into exactly that.
        let handler = unsafe { stub.read() };

        let slot = idt.wrapping_add(vector);
        // SAFETY: this core's own table, the table has 256 slots and this is
        // one of the first thirty-two, and it is not live until `lidt`.
        unsafe { slot.write(Gate::new(handler, ist)) };
    }

    // The two non-exception gates. The spurious one is not optional: the local
    // APIC has to be told a vector to deliver an unexplainable interrupt to,
    // and a machine with no gate there answers one with a fault about the
    // fault. The timer's is the first gate in this table that exists because
    // the kernel wants something to happen rather than because something might
    // go wrong.
    let devices = [
        (apic::TIMER_VECTOR as usize, (&raw const isr_timer) as u64),
        (apic::SPURIOUS_VECTOR as usize, (&raw const isr_spurious) as u64),
    ];
    for (vector, handler) in devices {
        let slot = idt.wrapping_add(vector);
        // SAFETY: this core's own table, both vectors came from a `u8` so both
        // are inside a table of 256, and none of it is live until `lidt` below.
        unsafe { slot.write(Gate::new(handler, 0)) };
    }

    let pointer = gdt::DescriptorPointer {
        limit: (core::mem::size_of::<[Gate; IDT_ENTRIES]>() - 1) as u16,
        base: idt as u64,
    };

    // SAFETY: the pointer describes the table built immediately above, in the
    // kernel window, which is mapped.
    unsafe {
        core::arch::asm!(
            "lidt [{ptr}]",
            ptr = in(reg) &pointer,
            options(readonly, nostack, preserves_flags),
        );
    }
}

/// The names the processor's manual uses, so a dump can be looked up.
fn vector_name(vector: u64) -> &'static str {
    match vector {
        0 => "divide error",
        1 => "debug",
        2 => "non-maskable interrupt",
        3 => "breakpoint",
        4 => "overflow",
        5 => "bound range exceeded",
        6 => "invalid opcode",
        7 => "device not available",
        8 => "double fault",
        10 => "invalid task state segment",
        11 => "segment not present",
        12 => "stack segment fault",
        13 => "general protection fault",
        14 => "page fault",
        16 => "x87 floating point",
        17 => "alignment check",
        18 => "machine check",
        19 => "simd floating point",
        21 => "control protection",
        _ => "reserved",
    }
}

/// The address a page fault was about.
fn faulting_address() -> u64 {
    let value: u64;
    // SAFETY: reading CR2 is a privileged register read with no side effect,
    // and the kernel runs at ring 0.
    unsafe {
        core::arch::asm!("mov {}, cr2", out(reg) value, options(nomem, nostack, preserves_flags));
    }
    value
}

/// Where every exception arrives.
///
/// # Safety
///
/// Called from the assembly stubs with `rdi` holding the stack pointer, which at
/// that moment is a fully populated [`Frame`]. Not to be called from Rust.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn interrupt_dispatch(frame: *mut Frame) {
    // SAFETY: the stubs pass the address of the frame they just pushed, which is
    // live for the duration of this call and aliased by nothing.
    let frame = unsafe { &mut *frame };

    // A breakpoint is a deliberate stop, not a failure. Returning from one is
    // also the only proof at M2 that the save-and-restore path is right: the
    // boot probe takes one, and the machine carries on.
    if frame.vector == BREAKPOINT {
        kprintln!("  exceptions    ok — breakpoint taken and returned");
        return;
    }

    // The timer. The one vector in this table that is not about something
    // having gone wrong, and the reason the stubs restore the frame and `iretq`
    // rather than only reporting: the interrupted code carries on, having lost
    // nothing but the time.
    if frame.vector == u64::from(apic::TIMER_VECTOR) {
        // SAFETY: reached from the timer's own gate, on the core the timer was
        // armed on, with interrupts disabled by that gate.
        unsafe { apic::on_tick() };
        return;
    }

    // An interrupt the local APIC withdrew between asserting it and having it
    // acknowledged. It is not an error, it needs no end-of-interrupt — the
    // architecture is explicit that sending one here would acknowledge somebody
    // else's interrupt — and there is nothing useful to say about it. Returning
    // silently is the whole handler.
    //
    // Silent on purpose rather than for lack of a counter: this path is
    // reachable from any interrupt, so anything it printed would appear in the
    // boot log at a moment nothing chose, and the boot log is a fixture.
    if frame.vector == u64::from(apic::SPURIOUS_VECTOR) {
        return;
    }

    report(frame);
    super::exit_qemu(super::Exit::Failure)
}

/// Print everything known about a fault.
fn report(frame: &Frame) {
    kprintln!();
    kprintln!("EXCEPTION {} — {}", frame.vector, vector_name(frame.vector));

    if frame.vector == PAGE_FAULT {
        let error = frame.error;
        kprintln!("  address       {:#018x}", faulting_address());
        kprintln!(
            "  cause         {} while {} in {} mode{}",
            if error & 1 == 0 { "not present" } else { "protection violation" },
            if error & 2 == 0 { "reading" } else { "writing" },
            if error & 4 == 0 { "kernel" } else { "user" },
            if error & 16 == 0 { "" } else { ", fetching an instruction" },
        );
    }

    kprintln!("  error         {:#018x}", frame.error);
    kprintln!("  rip           {:#018x}   cs {:#06x}", frame.rip, frame.cs);
    kprintln!("  rsp           {:#018x}   ss {:#06x}", frame.rsp, frame.ss);
    kprintln!("  rflags        {:#018x}", frame.rflags);
    kprintln!();

    let registers: [(&str, u64); 15] = [
        ("rax", frame.rax),
        ("rbx", frame.rbx),
        ("rcx", frame.rcx),
        ("rdx", frame.rdx),
        ("rsi", frame.rsi),
        ("rdi", frame.rdi),
        ("rbp", frame.rbp),
        ("r8 ", frame.r8),
        ("r9 ", frame.r9),
        ("r10", frame.r10),
        ("r11", frame.r11),
        ("r12", frame.r12),
        ("r13", frame.r13),
        ("r14", frame.r14),
        ("r15", frame.r15),
    ];

    let mut serial = super::serial::Serial;
    for (index, (name, value)) in registers.iter().enumerate() {
        let _ = write!(serial, "  {name} {value:#018x}");
        if index % 3 == 2 {
            let _ = writeln!(serial);
        }
    }
    let _ = writeln!(serial);
}

global_asm!(
    r#"
    .section .text, "ax", @progbits
    .code64

    // Thirty-two near-identical stubs, written out rather than generated by an
    // assembler macro. This is exactly the case a macro exists for, and it is
    // also the case where the macro's expansion rules become the thing being
    // debugged instead of the kernel. The ten vectors for which the processor
    // pushes an error code are the ones with no `pushq $0`.
isr_0:
    pushq $0
    pushq $0
    jmp isr_common
isr_1:
    pushq $0
    pushq $1
    jmp isr_common
isr_2:
    pushq $0
    pushq $2
    jmp isr_common
isr_3:
    pushq $0
    pushq $3
    jmp isr_common
isr_4:
    pushq $0
    pushq $4
    jmp isr_common
isr_5:
    pushq $0
    pushq $5
    jmp isr_common
isr_6:
    pushq $0
    pushq $6
    jmp isr_common
isr_7:
    pushq $0
    pushq $7
    jmp isr_common
isr_8:
    pushq $8
    jmp isr_common
isr_9:
    pushq $0
    pushq $9
    jmp isr_common
isr_10:
    pushq $10
    jmp isr_common
isr_11:
    pushq $11
    jmp isr_common
isr_12:
    pushq $12
    jmp isr_common
isr_13:
    pushq $13
    jmp isr_common
isr_14:
    pushq $14
    jmp isr_common
isr_15:
    pushq $0
    pushq $15
    jmp isr_common
isr_16:
    pushq $0
    pushq $16
    jmp isr_common
isr_17:
    pushq $17
    jmp isr_common
isr_18:
    pushq $0
    pushq $18
    jmp isr_common
isr_19:
    pushq $0
    pushq $19
    jmp isr_common
isr_20:
    pushq $0
    pushq $20
    jmp isr_common
isr_21:
    pushq $21
    jmp isr_common
isr_22:
    pushq $0
    pushq $22
    jmp isr_common
isr_23:
    pushq $0
    pushq $23
    jmp isr_common
isr_24:
    pushq $0
    pushq $24
    jmp isr_common
isr_25:
    pushq $0
    pushq $25
    jmp isr_common
isr_26:
    pushq $0
    pushq $26
    jmp isr_common
isr_27:
    pushq $0
    pushq $27
    jmp isr_common
isr_28:
    pushq $0
    pushq $28
    jmp isr_common
isr_29:
    pushq $29
    jmp isr_common
isr_30:
    pushq $30
    jmp isr_common
isr_31:
    pushq $0
    pushq $31
    jmp isr_common

    // Not exceptions, so not in the table above and not reached by index. The
    // processor pushes no error code for a device vector, so each stub pushes
    // the zero that keeps every frame the same shape, then its vector.
    .globl isr_timer
isr_timer:
    pushq $0
    pushq $32
    jmp isr_common

    .globl isr_spurious
isr_spurious:
    pushq $0
    pushq $255
    jmp isr_common

isr_common:
    // The direction flag is caller-saved and the interrupted code may have set
    // it. Everything below, including anything the handler calls, assumes it is
    // clear.
    cld

    pushq %rax
    pushq %rcx
    pushq %rdx
    pushq %rbx
    pushq %rbp
    pushq %rsi
    pushq %rdi
    pushq %r8
    pushq %r9
    pushq %r10
    pushq %r11
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15

    // The frame is the stack, so its address is the stack pointer.
    movq %rsp, %rdi
    call interrupt_dispatch

    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %r11
    popq %r10
    popq %r9
    popq %r8
    popq %rdi
    popq %rsi
    popq %rbp
    popq %rbx
    popq %rdx
    popq %rcx
    popq %rax

    // Discard the vector and the error code, which the processor did not push
    // and will not pop.
    addq $16, %rsp
    iretq

    // The table the descriptor table is built from, so the list of stubs exists
    // once rather than twice.
    .section .rodata, "a", @progbits
    .align 8
    .globl isr_table
isr_table:

    .quad isr_0
    .quad isr_1
    .quad isr_2
    .quad isr_3
    .quad isr_4
    .quad isr_5
    .quad isr_6
    .quad isr_7
    .quad isr_8
    .quad isr_9
    .quad isr_10
    .quad isr_11
    .quad isr_12
    .quad isr_13
    .quad isr_14
    .quad isr_15
    .quad isr_16
    .quad isr_17
    .quad isr_18
    .quad isr_19
    .quad isr_20
    .quad isr_21
    .quad isr_22
    .quad isr_23
    .quad isr_24
    .quad isr_25
    .quad isr_26
    .quad isr_27
    .quad isr_28
    .quad isr_29
    .quad isr_30
    .quad isr_31
"#,
    options(att_syntax)
);
