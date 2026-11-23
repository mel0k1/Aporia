#![no_std]
#![deny(unsafe_code)]

use ostd::{
    Error,
    arch::vm::{GuestContext, VcpuMsrs, VcpuRegs, VcpuSregs, VmxExitReason},
    mm::{CachePolicy, FrameAllocOptions, PAGE_SIZE, PageFlags, PageProperty, VmIo},
    power::{ExitCode, poweroff},
    prelude::*,
    task::{TaskOptions, disable_preempt},
    vm::{DummyGuestHooks, GuestMode, GuestPhysMemSpace, GuestReturnReason},
};

#[ostd::main]
fn main() {
    TaskOptions::new(|| {
        run_guest();
        poweroff(ExitCode::Success);
    })
    .spawn()
    .unwrap();
}

fn run_guest() {
    let guest_mode = match GuestMode::new() {
        Ok(guest_mode) => guest_mode,
        Err(Error::NotEnoughResources | Error::AccessDenied) => {
            println!("Error: VMX is not supported by the VM");
            return;
        }
        Err(err) => panic!("Failed to initialize VMX: {err:?}"),
    };

    let guest_mem = GuestPhysMemSpace::new().expect("EPT is required for guest memory");
    let frame = FrameAllocOptions::new().alloc_frame().unwrap();
    frame.write_bytes(0, GUEST_CODE).unwrap();
    {
        let guard = disable_preempt();
        let mut cursor = guest_mem.cursor_mut(&guard, &(0..PAGE_SIZE)).unwrap();
        cursor.map(
            frame.into(),
            PageProperty::new_guest(PageFlags::RWX, CachePolicy::Writeback),
        );
    }

    let mut context = GuestContext::new(
        VcpuRegs::new(),
        VcpuSregs::new_real_mode(),
        VcpuMsrs::default(),
    )
    .unwrap();

    loop {
        let exit = match guest_mode
            .execute(&mut context, &guest_mem, &DummyGuestHooks)
            .unwrap()
        {
            GuestReturnReason::VmExit(exit) => exit,
            GuestReturnReason::KernelEvent => continue,
        };

        match exit.exit_reason {
            reason
                if reason == VmxExitReason::IO_INSTRUCTION as u32
                    && exit.exit_qualification & 0x3f == 0
                    && (exit.exit_qualification >> 16) as u16 == 0x3f8 =>
            {
                print!("{}", char::from(context.regs().rax as u8));
                let regs = context.regs_mut();
                regs.rip = regs.rip.checked_add(exit.instruction_len as usize).unwrap();
            }
            reason if reason == VmxExitReason::HLT as u32 => break,
            reason => panic!("unsupported guest exit: {reason:#x}"),
        }
    }

    println!("Guest halted; returned to host.");
}

const GUEST_CODE: &[u8] = include_bytes!("../guest_hello");
