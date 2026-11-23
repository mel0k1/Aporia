use crate::nvme_spec::NvmeCommand;

#[repr(u8)]
enum AdminCommandSet {
    DeleteIosq = 0x00,
    CreateIosq = 0x01,
    DeleteIocq = 0x04,
    CreateIocq = 0x05,
    IdentifyCommand = 0x06,
}

#[repr(u8)]
enum IoCommandSet {
    Flush = 0x00,
    Write = 0x01,
    Read = 0x02,
}

const IO_CMD_NOT_FUSED_BITS: u8 = 6;

pub(crate) fn create_io_completion_queue(
    qid: u16,
    ptr: usize,
    size: u16,
    iv: Option<u16>,
) -> NvmeCommand {
    let cdw11 = if let Some(vector) = iv {
        ((vector as u32) << 16) | 0b11
    } else {
        0b1
    };

    NvmeCommand::from_raw_fields(
        AdminCommandSet::CreateIocq as u8,
        0,
        0,
        [ptr as u64, 0],
        [((size as u32) << 16) | (qid as u32), cdw11],
    )
}

pub(crate) fn create_io_submission_queue(
    qid: u16,
    ptr: usize,
    size: u16,
    cqid: u16,
) -> NvmeCommand {
    NvmeCommand::from_raw_fields(
        AdminCommandSet::CreateIosq as u8,
        0,
        0,
        [ptr as u64, 0],
        [
            ((size as u32) << 16) | (qid as u32),
            ((cqid as u32) << 16) | 1,
        ],
    )
}

#[expect(dead_code)]
pub(crate) fn delete_io_completion_queue(qid: u16) -> NvmeCommand {
    NvmeCommand::from_raw_fields(
        AdminCommandSet::DeleteIocq as u8,
        0,
        0,
        [0, 0],
        [qid as u32],
    )
}

#[expect(dead_code)]
pub(crate) fn delete_io_submission_queue(qid: u16) -> NvmeCommand {
    NvmeCommand::from_raw_fields(
        AdminCommandSet::DeleteIosq as u8,
        0,
        0,
        [0, 0],
        [qid as u32],
    )
}

pub(crate) fn identify_namespace(ptr: usize, nsid: u32) -> NvmeCommand {
    NvmeCommand::from_raw_fields(
        AdminCommandSet::IdentifyCommand as u8,
        0,
        nsid,
        [ptr as u64, 0],
        [],
    )
}

pub(crate) fn identify_controller(ptr: usize) -> NvmeCommand {
    NvmeCommand::from_raw_fields(
        AdminCommandSet::IdentifyCommand as u8,
        0,
        0,
        [ptr as u64, 0],
        [1],
    )
}

pub(crate) fn identify_namespace_list(ptr: usize, base: u32) -> NvmeCommand {
    NvmeCommand::from_raw_fields(
        AdminCommandSet::IdentifyCommand as u8,
        0,
        base,
        [ptr as u64, 0],
        [2],
    )
}

pub(crate) fn io_read(nsid: u32, lba: u64, nlb: u16, ptr0: u64, ptr1: u64) -> NvmeCommand {
    NvmeCommand::from_raw_fields(
        IoCommandSet::Read as u8,
        0 << IO_CMD_NOT_FUSED_BITS,
        nsid,
        [ptr0, ptr1],
        [lba as u32, (lba >> 32) as u32, nlb as u32],
    )
}

pub(crate) fn io_write(nsid: u32, lba: u64, nlb: u16, ptr0: u64, ptr1: u64) -> NvmeCommand {
    NvmeCommand::from_raw_fields(
        IoCommandSet::Write as u8,
        0 << IO_CMD_NOT_FUSED_BITS,
        nsid,
        [ptr0, ptr1],
        [lba as u32, (lba >> 32) as u32, nlb as u32],
    )
}

pub(crate) fn io_flush(nsid: u32) -> NvmeCommand {
    NvmeCommand::from_raw_fields(
        IoCommandSet::Flush as u8,
        0 << IO_CMD_NOT_FUSED_BITS,
        nsid,
        [0, 0],
        [],
    )
}
